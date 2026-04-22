use anyhow::Result;
use dirs::home_dir;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::collections::{hash_map::DefaultHasher, BTreeMap};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

pub const DEFAULT_CODEX_BASE_URL: &str = "https://api.openai.com/v1";
pub const DEFAULT_CODEX_MODEL: &str = "gpt-5.4";
pub const LEGACY_CODEX_BASE_URL: &str = "http://localhost:8080";
pub const LEGACY_CODEX_MODEL: &str = "codex";
pub const LEGACY_CODEX_MODEL_V1: &str = "gpt-5-codex";
pub const LEGACY_CODEX_MODEL_V1_MINI: &str = "gpt-5-codex-mini";

pub fn should_reset_codex_base_url(url: Option<&str>) -> bool {
    url.map(str::trim).map_or(true, |value| {
        value.is_empty() || value == LEGACY_CODEX_BASE_URL
    })
}

pub fn should_reset_codex_model(model: Option<&str>) -> bool {
    model.map(str::trim).map_or(true, |value| {
        value.is_empty()
            || matches!(
                value,
                LEGACY_CODEX_MODEL | LEGACY_CODEX_MODEL_V1 | LEGACY_CODEX_MODEL_V1_MINI
            )
    })
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ProviderConfigState {
    #[serde(
        default,
        serialize_with = "serialize_secret_option",
        deserialize_with = "deserialize_secret_option"
    )]
    pub api_key: Option<SecretString>,
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub revision: Option<String>,
    pub thinking: Option<bool>,
    pub num_ctx: Option<u32>,
}

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
    pub reasoning_effort: Option<String>,
    pub revision: Option<String>,
    pub thinking: Option<bool>,
    pub num_ctx: Option<u32>,
    pub system_prompt: Option<String>,
    pub system_prompt_file: Option<String>,
    pub append_system_prompt: Option<String>,
    pub append_system_prompt_file: Option<String>,
    pub compact_prompt: Option<String>,
    pub compact_prompt_file: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub provider_states: BTreeMap<String, ProviderConfigState>,
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
        self.sync_active_provider_state();
    }

    pub fn clear_api_key(&mut self) {
        self.api_key = None;
        self.sync_active_provider_state();
    }

    pub fn clear_provider_api_key(&mut self, provider: &str) {
        if self.provider == provider {
            self.clear_api_key();
            return;
        }
        if let Some(state) = self.provider_states.get_mut(provider) {
            state.api_key = None;
        }
    }

    pub fn set_provider(&mut self, provider: impl Into<String>) {
        self.sync_active_provider_state();
        self.provider = provider.into();
        self.reset_provider_scoped_fields();
        if let Some(state) = self.provider_states.get(&self.provider).cloned() {
            self.apply_provider_state(state);
        }
    }

    pub fn set_base_url(&mut self, value: Option<String>) {
        self.base_url = normalize_optional_string(value);
        self.sync_active_provider_state();
    }

    pub fn set_model(&mut self, value: Option<String>) {
        self.model = normalize_optional_string(value);
        self.sync_active_provider_state();
    }

    pub fn set_reasoning_effort(&mut self, value: Option<String>) {
        self.reasoning_effort = normalize_optional_string(value);
        self.sync_active_provider_state();
    }

    pub fn set_revision(&mut self, value: Option<String>) {
        self.revision = normalize_optional_string(value);
        self.sync_active_provider_state();
    }

    pub fn set_thinking(&mut self, value: Option<bool>) {
        self.thinking = value;
        self.sync_active_provider_state();
    }

    pub fn set_num_ctx(&mut self, value: Option<u32>) {
        self.num_ctx = value;
        self.sync_active_provider_state();
    }

    pub fn apply_codex_defaults(&mut self) {
        if should_reset_codex_base_url(self.base_url.as_deref()) {
            self.set_base_url(Some(DEFAULT_CODEX_BASE_URL.to_string()));
        }
        if should_reset_codex_model(self.model.as_deref()) {
            self.set_model(Some(DEFAULT_CODEX_MODEL.to_string()));
        }
    }

    fn sync_active_provider_state(&mut self) {
        if self.provider.trim().is_empty() {
            return;
        }
        self.provider_states
            .insert(self.provider.clone(), self.current_provider_state());
    }

    fn current_provider_state(&self) -> ProviderConfigState {
        ProviderConfigState {
            api_key: self.api_key.clone(),
            base_url: self.base_url.clone(),
            model: self.model.clone(),
            reasoning_effort: self.reasoning_effort.clone(),
            revision: self.revision.clone(),
            thinking: self.thinking,
            num_ctx: self.num_ctx,
        }
    }

    fn apply_provider_state(&mut self, state: ProviderConfigState) {
        self.api_key = state.api_key;
        self.base_url = state.base_url;
        self.model = state.model;
        self.reasoning_effort = state.reasoning_effort;
        self.revision = state.revision;
        self.thinking = state.thinking;
        self.num_ctx = state.num_ctx;
    }

    fn reset_provider_scoped_fields(&mut self) {
        self.api_key = None;
        self.base_url = None;
        self.model = None;
        self.reasoning_effort = None;
        self.revision = None;
        self.thinking = None;
        self.num_ctx = None;
    }
}

pub fn rara_home_dir() -> Result<PathBuf> {
    Ok(home_dir()
        .ok_or_else(|| anyhow::anyhow!("failed to resolve home directory for ~/.rara"))?
        .join(".rara"))
}

pub fn ensure_rara_home_dir() -> Result<PathBuf> {
    let rara_home = rara_home_dir()?;
    fs::create_dir_all(&rara_home)?;
    Ok(rara_home)
}

pub fn workspace_data_dir_for(root: &Path) -> Result<PathBuf> {
    let rara_home = ensure_rara_home_dir()?;
    workspace_data_dir_for_home(root, &rara_home)
}

pub fn workspace_data_dir_for_home(root: &Path, rara_home: &Path) -> Result<PathBuf> {
    let canonical_root = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let slug = workspace_slug(&canonical_root);
    let hash = stable_path_hash(&canonical_root);
    let dir = rara_home
        .join("workspaces")
        .join(format!("{slug}-{hash:016x}"));
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn workspace_slug(root: &Path) -> String {
    let raw = root
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("workspace");
    let mut slug = String::new();
    let mut last_dash = false;
    for ch in raw.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            last_dash = false;
        } else if !last_dash {
            slug.push('-');
            last_dash = true;
        }
    }
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        "workspace".to_string()
    } else {
        slug.chars().take(40).collect()
    }
}

fn stable_path_hash(root: &Path) -> u64 {
    let mut hasher = DefaultHasher::new();
    root.hash(&mut hasher);
    hasher.finish()
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn serialize_secret_option<S>(
    value: &Option<SecretString>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    Option::<String>::serialize(
        &value
            .as_ref()
            .map(|secret| secret.expose_secret().to_string()),
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
        Self::new_for_rara_home(ensure_rara_home_dir()?)
    }

    pub fn new_for_rara_home(rara_home: PathBuf) -> Result<Self> {
        fs::create_dir_all(&rara_home)?;
        Ok(Self {
            path: rara_home.join("config.json"),
        })
    }

    pub fn load(&self) -> Result<RaraConfig> {
        match fs::read_to_string(&self.path) {
            Ok(content) => serde_json::from_str(&content)
                .map_err(|err| anyhow::anyhow!("failed to parse {}: {err}", self.path.display())),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default_config()),
            Err(err) => Err(err.into()),
        }
    }

    pub fn save(&self, config: &RaraConfig) -> Result<()> {
        let content = serde_json::to_string_pretty(config)?;
        fs::write(&self.path, content)?;
        Ok(())
    }

    fn default_config() -> RaraConfig {
        RaraConfig {
            provider: "mock".to_string(),
            thinking: Some(true),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{workspace_data_dir_for_home, ConfigManager, RaraConfig};
    use std::fs;
    use tempfile::tempdir;

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

    #[test]
    fn provider_switch_restores_provider_specific_settings() {
        let mut config = RaraConfig {
            provider: "codex".to_string(),
            ..Default::default()
        };
        config.set_api_key("sk-codex");
        config.set_model(Some("codex".to_string()));
        config.set_reasoning_effort(Some("high".to_string()));
        config.set_base_url(Some("http://localhost:8080".to_string()));

        config.set_provider("ollama");
        assert_eq!(config.provider, "ollama");
        assert!(config.api_key().is_none());
        assert!(config.model.is_none());
        assert!(config.base_url.is_none());

        config.set_model(Some("qwen3".to_string()));
        config.set_base_url(Some("http://localhost:11434".to_string()));
        config.set_num_ctx(Some(32768));

        config.set_provider("codex");
        assert_eq!(config.api_key(), Some("sk-codex"));
        assert_eq!(config.model.as_deref(), Some("codex"));
        assert_eq!(config.reasoning_effort.as_deref(), Some("high"));
        assert_eq!(config.base_url.as_deref(), Some("http://localhost:8080"));
        assert_eq!(config.num_ctx, None);

        config.set_provider("ollama");
        assert_eq!(config.model.as_deref(), Some("qwen3"));
        assert_eq!(config.reasoning_effort, None);
        assert_eq!(config.base_url.as_deref(), Some("http://localhost:11434"));
        assert_eq!(config.num_ctx, Some(32768));
    }

    #[test]
    fn apply_codex_defaults_migrates_legacy_model_and_base_url() {
        let mut config = RaraConfig {
            provider: "codex".to_string(),
            ..Default::default()
        };
        config.set_model(Some("codex".to_string()));
        config.set_base_url(Some("http://localhost:8080".to_string()));

        config.apply_codex_defaults();

        assert_eq!(config.model.as_deref(), Some(super::DEFAULT_CODEX_MODEL));
        assert_eq!(
            config.base_url.as_deref(),
            Some(super::DEFAULT_CODEX_BASE_URL)
        );
    }

    #[test]
    fn config_manager_uses_rara_home() {
        let dir = tempdir().expect("tempdir");
        let manager =
            ConfigManager::new_for_rara_home(dir.path().join(".rara")).expect("config manager");
        assert_eq!(manager.path, dir.path().join(".rara").join("config.json"));
    }

    #[test]
    fn workspace_data_dir_lives_under_global_rara_home() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path().join("repo");
        fs::create_dir_all(&root).expect("mkdir root");

        let rara_home = temp.path().join(".rara-home");
        let data_dir = workspace_data_dir_for_home(&root, &rara_home).expect("workspace data dir");

        assert!(data_dir.starts_with(rara_home.join("workspaces")));
        assert!(data_dir.exists());
    }

    #[test]
    fn load_returns_error_for_invalid_json() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("config.json");
        fs::write(&path, "{invalid json").expect("write invalid config");
        let manager = ConfigManager { path };

        let err = manager.load().expect_err("invalid config should fail");
        assert!(err.to_string().contains("failed to parse"));
    }
}
