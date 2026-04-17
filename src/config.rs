use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct RaraConfig {
    pub provider: String,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub revision: Option<String>,
    pub thinking: Option<bool>,
    pub num_ctx: Option<u32>,
    pub system_prompt: Option<String>,
    pub system_prompt_file: Option<String>,
    pub append_system_prompt: Option<String>,
    pub append_system_prompt_file: Option<String>,
    pub compact_prompt: Option<String>,
    pub compact_prompt_file: Option<String>,
}

pub struct ConfigManager {
    pub path: PathBuf,
}

impl ConfigManager {
    pub fn new() -> Result<Self> {
        let rara_dir = std::env::current_dir()?.join(".rara");
        if !rara_dir.exists() {
            fs::create_dir_all(&rara_dir)?;
        }
        Ok(Self {
            path: rara_dir.join("config.json"),
        })
    }

    pub fn load(&self) -> RaraConfig {
        if let Ok(content) = fs::read_to_string(&self.path) {
            if let Ok(config) = serde_json::from_str(&content) {
                return config;
            }
        }
        RaraConfig {
            provider: "mock".to_string(),
            thinking: Some(true),
            ..Default::default()
        }
    }

    pub fn save(&self, config: &RaraConfig) -> Result<()> {
        let content = serde_json::to_string_pretty(config)?;
        fs::write(&self.path, content)?;
        Ok(())
    }
}
