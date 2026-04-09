use std::fs;
use std::path::{PathBuf};
use anyhow::{Result, anyhow};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub prompt: String,
}

pub struct SkillManager {
    pub skills: HashMap<String, Skill>,
}

impl SkillManager {
    pub fn new() -> Self {
        Self { skills: HashMap::new() }
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

    fn load_from_dir(&mut self, dir: &PathBuf) -> Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("md") {
                let name = path.file_stem().unwrap().to_string_lossy().to_string();
                let content = fs::read_to_string(&path)?;
                let description = content.lines().next().unwrap_or("No description").trim_start_matches("# ").to_string();
                
                self.skills.insert(name.clone(), Skill { name, description, prompt: content });
            }
        }
        Ok(())
    }

    pub fn get_skill(&self, name: &str) -> Option<&Skill> {
        self.skills.get(name)
    }

    pub fn list_skills(&self) -> Vec<(&String, &String)> {
        self.skills.iter().map(|(k, v)| (k, &v.description)).collect()
    }
}
