use crate::prompt::{PromptSource, PromptSourceKind};
use anyhow::Result;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

pub struct WorkspaceMemory {
    pub root: PathBuf,
    pub rara_dir: PathBuf,
    cache: Mutex<WorkspaceCache>,
}

#[derive(Default)]
struct WorkspaceCache {
    prompt_files: HashMap<PathBuf, CachedTextFile>,
    env_info: Option<CachedEnvInfo>,
}

#[derive(Clone)]
struct CachedTextFile {
    modified: Option<SystemTime>,
    content: String,
}

#[derive(Clone)]
struct CachedEnvInfo {
    git_head_marker: Option<SystemTime>,
    cwd: String,
    branch: String,
}

impl WorkspaceMemory {
    pub fn new() -> Result<Self> {
        let root = std::env::current_dir()?;
        let rara_dir = root.join(".rara");
        if !rara_dir.exists() {
            fs::create_dir_all(&rara_dir)?;
        }
        Ok(Self::from_paths(root, rara_dir))
    }

    pub fn from_paths(root: PathBuf, rara_dir: PathBuf) -> Self {
        Self {
            root,
            rara_dir,
            cache: Mutex::new(WorkspaceCache::default()),
        }
    }

    pub fn read_memory_file(&self) -> Option<String> {
        let path = self.rara_dir.join("memory.md");
        fs::read_to_string(path).ok()
    }

    pub fn write_memory_file(&self, content: &str) -> Result<()> {
        let path = self.rara_dir.join("memory.md");
        fs::write(path, content)?;
        Ok(())
    }

    pub fn discover_instructions(&self) -> Vec<String> {
        self.discover_prompt_sources()
            .into_iter()
            .filter(|source| {
                matches!(
                    source.kind,
                    PromptSourceKind::ProjectInstruction | PromptSourceKind::LocalInstruction
                )
            })
            .map(|source| format!("### {}:\n{}", source.label, source.content))
            .collect()
    }

    pub fn discover_prompt_sources(&self) -> Vec<PromptSource> {
        let mut sources = Vec::new();
        for file in ["CLAUDE.md", "GEMINI.md", "AGENTS.md"] {
            let path = self.root.join(file);
            if let Some(content) = self.cached_file_content(&path) {
                sources.push(PromptSource {
                    kind: PromptSourceKind::ProjectInstruction,
                    label: format!("Project Instruction ({file})"),
                    display_path: file.to_string(),
                    content,
                });
            }
        }
        let rara_inst = self.rara_dir.join("instructions.md");
        if let Some(content) = self.cached_file_content(&rara_inst) {
            sources.push(PromptSource {
                kind: PromptSourceKind::LocalInstruction,
                label: "RARA Local Instruction".to_string(),
                display_path: ".rara/instructions.md".to_string(),
                content,
            });
        }
        let memory = self.rara_dir.join("memory.md");
        if let Some(content) = self.cached_file_content(&memory) {
            sources.push(PromptSource {
                kind: PromptSourceKind::LocalMemory,
                label: "Local Project Memory".to_string(),
                display_path: ".rara/memory.md".to_string(),
                content,
            });
        }
        sources
    }

    pub fn get_env_info(&self) -> (String, String) {
        let cwd = self
            .root
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();
        let marker = self.git_head_marker();
        let mut cache = self.cache.lock().expect("workspace cache poisoned");
        if let Some(cached) = cache.env_info.as_ref() {
            if cached.cwd == cwd && cached.git_head_marker == marker {
                return (cached.cwd.clone(), cached.branch.clone());
            }
        }

        let branch = self.read_git_branch();
        cache.env_info = Some(CachedEnvInfo {
            git_head_marker: marker,
            cwd: cwd.clone(),
            branch: branch.clone(),
        });
        (cwd, branch)
    }

    fn cached_file_content(&self, path: &Path) -> Option<String> {
        let modified = fs::metadata(path).ok().and_then(|meta| meta.modified().ok());
        let mut cache = self.cache.lock().expect("workspace cache poisoned");
        if let Some(cached) = cache.prompt_files.get(path) {
            if cached.modified == modified {
                return Some(cached.content.clone());
            }
        }

        let content = fs::read_to_string(path).ok()?;
        cache.prompt_files.insert(
            path.to_path_buf(),
            CachedTextFile {
                modified,
                content: content.clone(),
            },
        );
        Some(content)
    }

    fn read_git_branch(&self) -> String {
        let Some(git_dir) = self.resolve_git_dir() else {
            return "no-git".to_string();
        };
        let head = git_dir.join("HEAD");
        let Ok(head_text) = fs::read_to_string(head) else {
            return "no-git".to_string();
        };
        let head_text = head_text.trim();
        if let Some(reference) = head_text.strip_prefix("ref: ") {
            return reference
                .rsplit('/')
                .next()
                .filter(|value| !value.is_empty())
                .unwrap_or("no-git")
                .to_string();
        }
        if head_text.is_empty() {
            "no-git".to_string()
        } else {
            format!("detached@{}", &head_text[..head_text.len().min(12)])
        }
    }

    fn git_head_marker(&self) -> Option<SystemTime> {
        let git_dir = self.resolve_git_dir()?;
        fs::metadata(git_dir.join("HEAD"))
            .ok()
            .and_then(|meta| meta.modified().ok())
    }

    fn resolve_git_dir(&self) -> Option<PathBuf> {
        let dot_git = self.root.join(".git");
        let metadata = fs::metadata(&dot_git).ok()?;
        if metadata.is_dir() {
            return Some(dot_git);
        }
        let raw = fs::read_to_string(&dot_git).ok()?;
        let value = raw.strip_prefix("gitdir:")?.trim();
        let git_dir = Path::new(value);
        if git_dir.is_absolute() {
            Some(git_dir.to_path_buf())
        } else {
            Some(self.root.join(git_dir))
        }
    }
}
