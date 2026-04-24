use anyhow::Result;
use dirs::home_dir;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, hash_map::DefaultHasher};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

pub const DEFAULT_CODEX_BASE_URL: &str = "https://api.openai.com/v1";
pub const DEFAULT_CODEX_MODEL: &str = "gpt-5.4";
pub const DEFAULT_CODEX_CHATGPT_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
pub const DEFAULT_REASONING_SUMMARY: &str = "auto";
pub const REASONING_SUMMARY_NONE: &str = "none";
pub const REASONING_SUMMARY_DETAILED: &str = "detailed";
pub const LEGACY_CODEX_BASE_URL: &str = "http://localhost:8080";
pub const LEGACY_CODEX_MODEL: &str = "codex";
pub const LEGACY_CODEX_MODEL_V1: &str = "gpt-5-codex";
pub const LEGACY_CODEX_MODEL_V1_MINI: &str = "gpt-5-codex-mini";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigValueSource {
    ProviderState,
    LegacyGlobal,
    BuiltInDefault,
    Unset,
}

impl ConfigValueSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::ProviderState => "provider_state",
            Self::LegacyGlobal => "legacy_global",
            Self::BuiltInDefault => "built_in_default",
            Self::Unset => "unset",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedProviderValue<'a> {
    pub value: Option<&'a str>,
    pub source: ConfigValueSource,
}

impl<'a> ResolvedProviderValue<'a> {
    pub fn display_or(self, fallback: &'a str) -> &'a str {
        self.value.unwrap_or(fallback)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectiveProviderSurface<'a> {
    pub provider: &'a str,
    pub model: ResolvedProviderValue<'a>,
    pub base_url: ResolvedProviderValue<'a>,
    pub revision: ResolvedProviderValue<'a>,
    pub reasoning_effort: ResolvedProviderValue<'a>,
    pub reasoning_summary: ResolvedProviderValue<'a>,
    pub api_key: ResolvedProviderValue<'a>,
}

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

pub fn should_apply_codex_base_url(url: Option<&str>, expected: &str) -> bool {
    url.map(str::trim).map_or(true, |value| {
        value.is_empty()
            || value == LEGACY_CODEX_BASE_URL
            || ((value == DEFAULT_CODEX_BASE_URL || value == DEFAULT_CODEX_CHATGPT_BASE_URL)
                && value != expected)
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
    pub reasoning_summary: Option<String>,
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
    pub reasoning_summary: Option<String>,
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

    pub fn set_reasoning_summary(&mut self, value: Option<String>) {
        self.reasoning_summary = normalize_reasoning_summary(value);
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
        self.apply_codex_defaults_for_base_url(DEFAULT_CODEX_BASE_URL);
    }

    pub fn apply_codex_defaults_for_base_url(&mut self, base_url: &str) {
        if should_apply_codex_base_url(self.base_url.as_deref(), base_url) {
            self.set_base_url(Some(base_url.to_string()));
        }
        if should_reset_codex_model(self.model.as_deref()) {
            self.set_model(Some(DEFAULT_CODEX_MODEL.to_string()));
        }
    }

    pub fn migrate_legacy_provider_state(&mut self) {
        self.reasoning_summary =
            migrate_reasoning_summary(self.reasoning_summary.take(), self.thinking);
        for state in self.provider_states.values_mut() {
            state.reasoning_summary =
                migrate_reasoning_summary(state.reasoning_summary.take(), state.thinking);
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
            reasoning_summary: self.reasoning_summary.clone(),
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
        self.reasoning_summary = state.reasoning_summary;
        self.revision = state.revision;
        self.thinking = state.thinking;
        self.num_ctx = state.num_ctx;
    }

    fn reset_provider_scoped_fields(&mut self) {
        self.api_key = None;
        self.base_url = None;
        self.model = None;
        self.reasoning_effort = None;
        self.reasoning_summary = Some(DEFAULT_REASONING_SUMMARY.to_string());
        self.revision = None;
        self.thinking = None;
        self.num_ctx = None;
    }

    pub fn model_source(&self) -> ConfigValueSource {
        provider_scoped_string_source(
            self.provider_states.get(&self.provider),
            self.model.as_deref(),
            |state| state.model.as_deref(),
            false,
        )
    }

    pub fn reasoning_summary_source(&self) -> ConfigValueSource {
        let provider_state = self.provider_states.get(&self.provider);
        if let Some(state) = provider_state {
            if self.reasoning_summary.as_deref() == state.reasoning_summary.as_deref() {
                return match self.reasoning_summary.as_deref() {
                    Some(_) => ConfigValueSource::ProviderState,
                    None => ConfigValueSource::BuiltInDefault,
                };
            }
        }

        match self.reasoning_summary.as_deref() {
            Some(DEFAULT_REASONING_SUMMARY) | None => ConfigValueSource::BuiltInDefault,
            Some(_) => ConfigValueSource::LegacyGlobal,
        }
    }

    pub fn base_url_source(&self) -> ConfigValueSource {
        provider_scoped_string_source(
            self.provider_states.get(&self.provider),
            self.base_url.as_deref(),
            |state| state.base_url.as_deref(),
            false,
        )
    }

    pub fn revision_source(&self) -> ConfigValueSource {
        provider_scoped_string_source(
            self.provider_states.get(&self.provider),
            self.revision.as_deref(),
            |state| state.revision.as_deref(),
            false,
        )
    }

    pub fn reasoning_effort_source(&self) -> ConfigValueSource {
        provider_scoped_string_source(
            self.provider_states.get(&self.provider),
            self.reasoning_effort.as_deref(),
            |state| state.reasoning_effort.as_deref(),
            false,
        )
    }

    pub fn api_key_source(&self) -> ConfigValueSource {
        provider_scoped_secret_source(
            self.provider_states.get(&self.provider),
            self.api_key.as_ref(),
            |state| state.api_key.as_ref(),
        )
    }

    pub fn effective_provider_surface(&self) -> EffectiveProviderSurface<'_> {
        EffectiveProviderSurface {
            provider: &self.provider,
            model: ResolvedProviderValue {
                value: self.model.as_deref(),
                source: self.model_source(),
            },
            base_url: ResolvedProviderValue {
                value: self.base_url.as_deref(),
                source: self.base_url_source(),
            },
            revision: ResolvedProviderValue {
                value: self.revision.as_deref(),
                source: self.revision_source(),
            },
            reasoning_effort: ResolvedProviderValue {
                value: self.reasoning_effort.as_deref(),
                source: self.reasoning_effort_source(),
            },
            reasoning_summary: ResolvedProviderValue {
                value: self.reasoning_summary.as_deref(),
                source: self.reasoning_summary_source(),
            },
            api_key: ResolvedProviderValue {
                value: self.api_key(),
                source: self.api_key_source(),
            },
        }
    }
}

fn provider_scoped_string_source<'a, F>(
    provider_state: Option<&'a ProviderConfigState>,
    current: Option<&'a str>,
    provider_getter: F,
    has_built_in_default: bool,
) -> ConfigValueSource
where
    F: Fn(&'a ProviderConfigState) -> Option<&'a str>,
{
    if let Some(state) = provider_state {
        if current == provider_getter(state) {
            return match current {
                Some(_) => ConfigValueSource::ProviderState,
                None if has_built_in_default => ConfigValueSource::BuiltInDefault,
                None => ConfigValueSource::Unset,
            };
        }
    }

    match current {
        Some(_) => ConfigValueSource::LegacyGlobal,
        None if has_built_in_default => ConfigValueSource::BuiltInDefault,
        None => ConfigValueSource::Unset,
    }
}

fn provider_scoped_secret_source<'a, F>(
    provider_state: Option<&'a ProviderConfigState>,
    current: Option<&'a SecretString>,
    provider_getter: F,
) -> ConfigValueSource
where
    F: Fn(&'a ProviderConfigState) -> Option<&'a SecretString>,
{
    let current = current.map(SecretString::expose_secret);
    if let Some(state) = provider_state {
        let provider_value = provider_getter(state).map(SecretString::expose_secret);
        if current == provider_value {
            return match current {
                Some(value) if !value.trim().is_empty() => ConfigValueSource::ProviderState,
                _ => ConfigValueSource::Unset,
            };
        }
    }

    match current {
        Some(value) if !value.trim().is_empty() => ConfigValueSource::LegacyGlobal,
        _ => ConfigValueSource::Unset,
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

fn normalize_reasoning_summary(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let normalized = value.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "" => None,
            DEFAULT_REASONING_SUMMARY | REASONING_SUMMARY_NONE | REASONING_SUMMARY_DETAILED => {
                Some(normalized)
            }
            _ => Some(DEFAULT_REASONING_SUMMARY.to_string()),
        }
    })
}

fn migrate_reasoning_summary(
    reasoning_summary: Option<String>,
    legacy_thinking: Option<bool>,
) -> Option<String> {
    normalize_reasoning_summary(reasoning_summary).or_else(|| {
        Some(match legacy_thinking {
            Some(false) => REASONING_SUMMARY_NONE.to_string(),
            Some(true) | None => DEFAULT_REASONING_SUMMARY.to_string(),
        })
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
            Ok(content) => {
                let mut config: RaraConfig = serde_json::from_str(&content).map_err(|err| {
                    anyhow::anyhow!("failed to parse {}: {err}", self.path.display())
                })?;
                config.migrate_legacy_provider_state();
                Ok(config)
            }
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
            reasoning_summary: Some(DEFAULT_REASONING_SUMMARY.to_string()),
            thinking: Some(true),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ConfigManager, RaraConfig, workspace_data_dir_for_home};
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
        config.set_reasoning_summary(Some("detailed".to_string()));
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
        assert_eq!(config.reasoning_summary.as_deref(), Some("detailed"));
        assert_eq!(config.base_url.as_deref(), Some("http://localhost:8080"));
        assert_eq!(config.num_ctx, None);

        config.set_provider("ollama");
        assert_eq!(config.model.as_deref(), Some("qwen3"));
        assert_eq!(config.reasoning_effort, None);
        assert_eq!(
            config.reasoning_summary.as_deref(),
            Some(super::DEFAULT_REASONING_SUMMARY)
        );
        assert_eq!(config.base_url.as_deref(), Some("http://localhost:11434"));
        assert_eq!(config.num_ctx, Some(32768));
    }

    #[test]
    fn load_migrates_legacy_thinking_to_reasoning_summary() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("config.json");
        fs::write(
            &path,
            r#"{
  "provider": "codex",
  "thinking": false,
  "provider_states": {
    "codex": {
      "thinking": true
    }
  }
}"#,
        )
        .expect("write config");
        let manager = ConfigManager { path };

        let config = manager.load().expect("load config");

        assert_eq!(config.reasoning_summary.as_deref(), Some(super::REASONING_SUMMARY_NONE));
        assert_eq!(
            config.provider_states["codex"].reasoning_summary.as_deref(),
            Some(super::DEFAULT_REASONING_SUMMARY)
        );
    }

    #[test]
    fn invalid_reasoning_summary_normalizes_to_auto() {
        let mut config = RaraConfig::default();
        config.set_reasoning_summary(Some("verbose".to_string()));

        assert_eq!(
            config.reasoning_summary.as_deref(),
            Some(super::DEFAULT_REASONING_SUMMARY)
        );
    }

    #[test]
    fn reports_provider_and_default_value_sources() {
        let mut config = RaraConfig {
            provider: "openai-compatible".to_string(),
            ..Default::default()
        };
        config.set_model(Some("custom-model".to_string()));
        config.set_base_url(Some("http://proxy.local/v1".to_string()));
        config.set_reasoning_summary(Some("detailed".to_string()));

        assert_eq!(config.model_source(), super::ConfigValueSource::ProviderState);
        assert_eq!(config.base_url_source(), super::ConfigValueSource::ProviderState);
        assert_eq!(
            config.reasoning_summary_source(),
            super::ConfigValueSource::ProviderState
        );

        let mut defaulted = RaraConfig::default();
        defaulted.provider = "mock".to_string();
        defaulted.reasoning_summary = Some(super::DEFAULT_REASONING_SUMMARY.to_string());
        assert_eq!(
            defaulted.reasoning_summary_source(),
            super::ConfigValueSource::BuiltInDefault
        );
        assert_eq!(defaulted.model_source(), super::ConfigValueSource::Unset);
    }

    #[test]
    fn effective_provider_surface_reports_values_and_sources() {
        let mut config = RaraConfig {
            provider: "openai-compatible".to_string(),
            ..Default::default()
        };
        config.set_api_key("sk-test");
        config.set_model(Some("custom-model".to_string()));
        config.set_base_url(Some("http://proxy.local/v1".to_string()));
        config.set_reasoning_effort(Some("high".to_string()));
        config.set_reasoning_summary(Some("detailed".to_string()));

        let surface = config.effective_provider_surface();
        assert_eq!(surface.provider, "openai-compatible");
        assert_eq!(surface.model.value, Some("custom-model"));
        assert_eq!(surface.model.source, super::ConfigValueSource::ProviderState);
        assert_eq!(surface.base_url.value, Some("http://proxy.local/v1"));
        assert_eq!(surface.base_url.source, super::ConfigValueSource::ProviderState);
        assert_eq!(surface.reasoning_effort.value, Some("high"));
        assert_eq!(
            surface.reasoning_effort.source,
            super::ConfigValueSource::ProviderState
        );
        assert_eq!(surface.reasoning_summary.value, Some("detailed"));
        assert_eq!(
            surface.reasoning_summary.source,
            super::ConfigValueSource::ProviderState
        );
        assert_eq!(surface.api_key.source, super::ConfigValueSource::ProviderState);
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
    fn apply_codex_defaults_for_base_url_switches_between_known_codex_defaults() {
        let mut config = RaraConfig {
            provider: "codex".to_string(),
            base_url: Some(super::DEFAULT_CODEX_BASE_URL.to_string()),
            model: Some(super::DEFAULT_CODEX_MODEL.to_string()),
            ..Default::default()
        };

        config.apply_codex_defaults_for_base_url(super::DEFAULT_CODEX_CHATGPT_BASE_URL);

        assert_eq!(
            config.base_url.as_deref(),
            Some(super::DEFAULT_CODEX_CHATGPT_BASE_URL)
        );
        assert_eq!(config.model.as_deref(), Some(super::DEFAULT_CODEX_MODEL));
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
