use anyhow::Result;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct RaraConfig {
    pub provider: String,
    #[serde(
        default,
        serialize_with = "serialize_secret_option",
        deserialize_with = "deserialize_secret_option"
    )]
    pub api_key: Option<SecretString>,
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

impl RaraConfig {
    pub fn api_key(&self) -> Option<&str> {
        self.api_key.as_ref().map(SecretString::expose_secret)
    }

    pub fn has_api_key(&self) -> bool {
        self.api_key().is_some_and(|value| !value.is_empty())
    }

    pub fn set_api_key(&mut self, value: impl Into<String>) {
        self.api_key = Some(SecretString::from(value.into()));
    }

    pub fn clear_api_key(&mut self) {
        self.api_key = None;
    }
}

fn serialize_secret_option<S>(
    value: &Option<SecretString>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    Option::<String>::serialize(
        &value.as_ref().map(|secret| secret.expose_secret().to_string()),
        serializer,
    )
}

fn deserialize_secret_option<'de, D>(deserializer: D) -> Result<Option<SecretString>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    Ok(value.map(SecretString::from))
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

#[cfg(test)]
mod tests {
    use super::RaraConfig;

    #[test]
    fn secret_api_key_roundtrips_through_json() {
        let mut config = RaraConfig {
            provider: "codex".to_string(),
            ..Default::default()
        };
        config.set_api_key("sk-test-value");

        let json = serde_json::to_string(&config).expect("serialize config");
        let restored: RaraConfig = serde_json::from_str(&json).expect("deserialize config");

        assert_eq!(restored.api_key(), Some("sk-test-value"));
        assert!(restored.has_api_key());
    }

    #[test]
    fn empty_secret_is_not_counted_as_configured() {
        let mut config = RaraConfig::default();
        config.set_api_key("");
        assert!(!config.has_api_key());
    }
}
