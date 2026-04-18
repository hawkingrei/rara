use anyhow::{anyhow, Result};
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

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
        let global_dir = home.join(".rara/skills");
        let local_dir = std::env::current_dir()?.join(".rara/skills");

        for dir in &[global_dir, local_dir] {
            if dir.exists() {
                self.load_from_dir(dir)?;
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
    use super::SkillManager;
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
}
