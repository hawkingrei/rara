use crate::prompt::{PromptSource, PromptSourceKind};
use anyhow::Result;
use std::fs;
use std::path::PathBuf;

pub struct WorkspaceMemory {
    pub root: PathBuf,
    pub rara_dir: PathBuf,
}

impl WorkspaceMemory {
    pub fn new() -> Result<Self> {
        let root = std::env::current_dir()?;
        let rara_dir = root.join(".rara");
        if !rara_dir.exists() {
            fs::create_dir_all(&rara_dir)?;
        }
        Ok(Self { root, rara_dir })
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
            if let Ok(content) = fs::read_to_string(&path) {
                sources.push(PromptSource {
                    kind: PromptSourceKind::ProjectInstruction,
                    label: format!("Project Instruction ({file})"),
                    display_path: file.to_string(),
                    content,
                });
            }
        }
        let rara_inst = self.rara_dir.join("instructions.md");
        if let Ok(content) = fs::read_to_string(&rara_inst) {
            sources.push(PromptSource {
                kind: PromptSourceKind::LocalInstruction,
                label: "RARA Local Instruction".to_string(),
                display_path: ".rara/instructions.md".to_string(),
                content,
            });
        }
        let memory = self.rara_dir.join("memory.md");
        if let Ok(content) = fs::read_to_string(&memory) {
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
        let cwd = self.root.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();
        
        let branch = std::process::Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .ok()
            .and_then(|out| String::from_utf8(out.stdout).ok())
            .unwrap_or_else(|| "no-git".to_string())
            .trim()
            .to_string();

        (cwd, branch)
    }
}
