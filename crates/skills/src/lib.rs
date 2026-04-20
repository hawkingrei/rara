use anyhow::{anyhow, Result};
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub prompt: String,
    pub display_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillSummary {
    pub name: String,
    pub description: String,
    pub display_path: String,
}

pub struct SkillManager {
    pub skills: HashMap<String, Skill>,
}

impl SkillManager {
    pub fn new() -> Self {
        Self {
            skills: HashMap::new(),
        }
    }

    pub fn load_all(&mut self) -> Result<()> {
        let home = dirs::home_dir().ok_or_else(|| anyhow!("No home dir"))?;
        let cwd = std::env::current_dir()?;
        for dir in skill_search_dirs(&home, &cwd) {
            if dir.exists() {
                self.load_from_dir(&dir)?;
            }
        }
        Ok(())
    }

    pub fn load_from_dir(&mut self, dir: &Path) -> Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                let skill_file = path.join("SKILL.md");
                if skill_file.exists() {
                    self.load_skill_file(&skill_file, dir)?;
                }
                continue;
            }

            if path.extension().and_then(|s| s.to_str()) == Some("md") {
                self.load_skill_file(&path, dir)?;
            }
        }
        Ok(())
    }

    fn load_skill_file(&mut self, path: &Path, base_dir: &Path) -> Result<()> {
        let content = fs::read_to_string(path)?;
        let name = skill_name_from_path(path);
        let description = content
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("No description")
            .trim_start_matches("# ")
            .to_string();
        let display_path = path
            .strip_prefix(base_dir)
            .unwrap_or(path)
            .display()
            .to_string();

        self.skills.insert(
            name.clone(),
            Skill {
                name,
                description,
                prompt: content,
                display_path,
            },
        );
        Ok(())
    }

    pub fn get_skill(&self, name: &str) -> Option<&Skill> {
        self.skills.get(name)
    }

    pub fn invoke_instructions(&self, name: &str) -> Result<String> {
        self.get_skill(name)
            .map(|skill| skill.prompt.clone())
            .ok_or_else(|| anyhow!("Skill not found: {name}"))
    }

    pub fn list_summaries(&self) -> Vec<SkillSummary> {
        let mut items = self
            .skills
            .values()
            .map(|skill| SkillSummary {
                name: skill.name.clone(),
                description: skill.description.clone(),
                display_path: skill.display_path.clone(),
            })
            .collect::<Vec<_>>();
        items.sort_by(|a, b| a.name.cmp(&b.name));
        items
    }
}

fn skill_search_dirs(home: &Path, cwd: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![
        home.join(".rara/skills"),
        home.join(".agents/skills"),
        home.join(".codex/skills"),
    ];

    for dir in repo_skill_search_dirs(cwd) {
        dirs.push(dir.join(".agents/skills"));
    }

    dirs.push(cwd.join(".rara/skills"));
    dedupe_paths(&mut dirs);
    dirs
}

fn repo_skill_search_dirs(cwd: &Path) -> Vec<PathBuf> {
    let root = find_project_root(cwd).unwrap_or_else(|| cwd.to_path_buf());
    let mut dirs = Vec::new();
    let mut current = Some(cwd);
    while let Some(dir) = current {
        dirs.push(dir.to_path_buf());
        if dir == root {
            break;
        }
        current = dir.parent();
    }
    dirs.reverse();
    dirs
}

fn find_project_root(start: &Path) -> Option<PathBuf> {
    let mut current = Some(start);
    while let Some(dir) = current {
        if dir.join(".git").exists() {
            return Some(dir.to_path_buf());
        }
        current = dir.parent();
    }
    None
}

fn dedupe_paths(paths: &mut Vec<PathBuf>) {
    let mut seen = Vec::<PathBuf>::new();
    paths.retain(|path| {
        if seen.iter().any(|existing| existing == path) {
            false
        } else {
            seen.push(path.clone());
            true
        }
    });
}

fn skill_name_from_path(path: &Path) -> String {
    if path.file_name().and_then(|value| value.to_str()) == Some("SKILL.md") {
        path.parent()
            .and_then(|value| value.file_name())
            .and_then(|value| value.to_str())
            .unwrap_or("unknown")
            .to_string()
    } else {
        path.file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("unknown")
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{repo_skill_search_dirs, skill_search_dirs, SkillManager};
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn loads_legacy_markdown_skills() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("legacy.md"), "# Legacy Skill\nbody").expect("write");

        let mut manager = SkillManager::new();
        manager.load_from_dir(dir.path()).expect("load");

        let skill = manager.get_skill("legacy").expect("legacy skill");
        assert_eq!(skill.description, "Legacy Skill");
        assert_eq!(skill.display_path, "legacy.md");
    }

    #[test]
    fn loads_directory_skills_from_skill_md() {
        let dir = tempdir().expect("tempdir");
        let skill_dir = dir.path().join("reviewer");
        fs::create_dir_all(&skill_dir).expect("mkdir");
        fs::write(skill_dir.join("SKILL.md"), "# Reviewer\nworkflow").expect("write");

        let mut manager = SkillManager::new();
        manager.load_from_dir(dir.path()).expect("load");

        let skill = manager.get_skill("reviewer").expect("reviewer skill");
        assert_eq!(skill.description, "Reviewer");
        assert_eq!(skill.display_path, "reviewer/SKILL.md");
    }

    #[test]
    fn search_dirs_include_codex_compatible_home_and_repo_roots() {
        let temp = tempdir().expect("tempdir");
        let home = temp.path().join("home");
        let repo = temp.path().join("repo");
        let nested = repo.join("crates/skills");
        fs::create_dir_all(&home).expect("mkdir home");
        fs::create_dir_all(&nested).expect("mkdir nested");
        fs::create_dir_all(repo.join(".git")).expect("mkdir git");

        let dirs = skill_search_dirs(&home, &nested);
        assert!(dirs.contains(&home.join(".rara/skills")));
        assert!(dirs.contains(&home.join(".agents/skills")));
        assert!(dirs.contains(&home.join(".codex/skills")));
        assert!(dirs.contains(&repo.join(".agents/skills")));
        assert!(dirs.contains(&repo.join("crates/.agents/skills")));
        assert!(dirs.contains(&nested.join(".rara/skills")));
    }

    #[test]
    fn repo_skill_search_dirs_walks_from_project_root_to_cwd() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        let nested = repo.join("a/b");
        fs::create_dir_all(&nested).expect("mkdir nested");
        fs::create_dir_all(repo.join(".git")).expect("mkdir git");

        let dirs = repo_skill_search_dirs(&nested);
        assert_eq!(dirs, vec![repo.clone(), repo.join("a"), nested]);
    }
}
