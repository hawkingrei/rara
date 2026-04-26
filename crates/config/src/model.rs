use anyhow::Result;
use dirs::home_dir;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::collections::{hash_map::DefaultHasher, BTreeMap};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use crate::defaults::{
    should_apply_codex_base_url, should_reset_codex_model, DEFAULT_CODEX_BASE_URL,
    DEFAULT_CODEX_MODEL, DEFAULT_DEEPSEEK_BASE_URL, DEFAULT_DEEPSEEK_MODEL, DEFAULT_KIMI_BASE_URL,
    DEFAULT_KIMI_MODEL, DEFAULT_OPENAI_COMPATIBLE_BASE_URL, DEFAULT_OPENAI_COMPATIBLE_MODEL,
    DEFAULT_OPENROUTER_BASE_URL, DEFAULT_OPENROUTER_MODEL, DEFAULT_REASONING_SUMMARY,
};
use crate::migration::migrate_reasoning_summary;
use crate::provider_surface::{ConfigValueSource, EffectiveProviderSurface, ResolvedProviderValue};
use crate::secrets::{deserialize_secret_option, serialize_secret_option};
use crate::serde_helpers::{normalize_optional_string, normalize_reasoning_summary};

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

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum OpenAiEndpointKind {
    Custom,
    Deepseek,
    Kimi,
    Openrouter,
}

impl OpenAiEndpointKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Custom => "Custom endpoint",
            Self::Deepseek => "DeepSeek",
            Self::Kimi => "Kimi",
            Self::Openrouter => "OpenRouter",
        }
    }

    pub fn default_profile_id(self) -> &'static str {
        match self {
            Self::Custom => "custom-default",
            Self::Deepseek => "deepseek-default",
            Self::Kimi => "kimi-default",
            Self::Openrouter => "openrouter-default",
        }
    }

    pub fn default_base_url(self) -> &'static str {
        match self {
            Self::Custom => DEFAULT_OPENAI_COMPATIBLE_BASE_URL,
            Self::Deepseek => DEFAULT_DEEPSEEK_BASE_URL,
            Self::Kimi => DEFAULT_KIMI_BASE_URL,
            Self::Openrouter => DEFAULT_OPENROUTER_BASE_URL,
        }
    }

    pub fn default_model(self) -> &'static str {
        match self {
            Self::Custom => DEFAULT_OPENAI_COMPATIBLE_MODEL,
            Self::Deepseek => DEFAULT_DEEPSEEK_MODEL,
            Self::Kimi => DEFAULT_KIMI_MODEL,
            Self::Openrouter => DEFAULT_OPENROUTER_MODEL,
        }
    }

    fn from_legacy_provider(provider: &str) -> Option<Self> {
        match provider {
            "openai-compatible" => Some(Self::Custom),
            "deepseek" => Some(Self::Deepseek),
            "kimi" => Some(Self::Kimi),
            "openrouter" => Some(Self::Openrouter),
            _ => None,
        }
    }
}

impl Default for OpenAiEndpointKind {
    fn default() -> Self {
        Self::Custom
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct OpenAiEndpointProfile {
    pub id: String,
    pub label: String,
    pub kind: OpenAiEndpointKind,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_openai_profile_id: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub openai_profiles: BTreeMap<String, OpenAiEndpointProfile>,
}

impl RaraConfig {
    pub fn is_openai_compatible_family(provider: &str) -> bool {
        OpenAiEndpointKind::from_legacy_provider(provider).is_some()
    }

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
        if let Some(kind) = OpenAiEndpointKind::from_legacy_provider(provider) {
            if self.provider == provider {
                self.clear_api_key();
            } else if self.provider == "openai-compatible"
                && self.active_openai_profile_kind() == Some(kind)
            {
                self.clear_api_key();
            } else if let Some(profile) = self.openai_profiles.get_mut(kind.default_profile_id()) {
                profile.api_key = None;
            }
            return;
        }
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
        let provider = provider.into();
        if let Some(kind) = OpenAiEndpointKind::from_legacy_provider(provider.as_str()) {
            self.provider = "openai-compatible".to_string();
            self.reset_provider_scoped_fields();
            let profile = self.profile_for_kind_or_default(kind);
            self.active_openai_profile_id = Some(profile.id.clone());
            self.openai_profiles
                .insert(profile.id.clone(), profile.clone());
            self.apply_openai_profile(profile);
            return;
        }
        self.provider = provider;
        self.reset_provider_scoped_fields();
        if self.provider == "openai-compatible" {
            let profile = self
                .active_openai_profile()
                .cloned()
                .unwrap_or_else(|| self.profile_for_kind_or_default(OpenAiEndpointKind::Custom));
            self.active_openai_profile_id = Some(profile.id.clone());
            self.openai_profiles
                .insert(profile.id.clone(), profile.clone());
            self.apply_openai_profile(profile);
        } else if let Some(state) = self.provider_states.get(&self.provider).cloned() {
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
        self.migrate_legacy_openai_profiles();
    }

    pub fn effective_provider_surface(&self) -> EffectiveProviderSurface<'_> {
        let provider_state = if self.provider == "openai-compatible" {
            None
        } else {
            self.provider_states.get(&self.provider)
        };
        let profile = if self.provider == "openai-compatible" {
            self.active_openai_profile()
        } else {
            None
        };
        EffectiveProviderSurface {
            provider: self.provider.as_str(),
            model: resolve_provider_value(
                provider_state
                    .and_then(|state| state.model.as_deref())
                    .or_else(|| profile.and_then(|profile| profile.model.as_deref())),
                self.model.as_deref(),
                None,
            ),
            base_url: resolve_provider_value(
                provider_state
                    .and_then(|state| state.base_url.as_deref())
                    .or_else(|| profile.and_then(|profile| profile.base_url.as_deref())),
                self.base_url.as_deref(),
                None,
            ),
            revision: resolve_provider_value(
                provider_state
                    .and_then(|state| state.revision.as_deref())
                    .or_else(|| profile.and_then(|profile| profile.revision.as_deref())),
                self.revision.as_deref(),
                None,
            ),
            reasoning_effort: resolve_provider_value(
                provider_state
                    .and_then(|state| state.reasoning_effort.as_deref())
                    .or_else(|| profile.and_then(|profile| profile.reasoning_effort.as_deref())),
                self.reasoning_effort.as_deref(),
                None,
            ),
            reasoning_summary: resolve_provider_value(
                provider_state
                    .and_then(|state| state.reasoning_summary.as_deref())
                    .or_else(|| profile.and_then(|profile| profile.reasoning_summary.as_deref())),
                self.reasoning_summary.as_deref(),
                Some(DEFAULT_REASONING_SUMMARY),
            ),
            api_key: resolve_provider_value(
                provider_state
                    .and_then(|state| state.api_key.as_ref().map(SecretString::expose_secret))
                    .or_else(|| {
                        profile.and_then(|profile| {
                            profile.api_key.as_ref().map(SecretString::expose_secret)
                        })
                    }),
                self.api_key.as_ref().map(SecretString::expose_secret),
                None,
            ),
        }
    }

    pub fn active_openai_profile_id(&self) -> Option<&str> {
        self.active_openai_profile_id
            .as_deref()
            .filter(|id| self.openai_profiles.contains_key(*id))
            .or_else(|| self.openai_profiles.keys().next().map(String::as_str))
    }

    pub fn active_openai_profile(&self) -> Option<&OpenAiEndpointProfile> {
        let id = self.active_openai_profile_id()?;
        self.openai_profiles.get(id)
    }

    pub fn active_openai_profile_label(&self) -> Option<&str> {
        self.active_openai_profile()
            .map(|profile| profile.label.as_str())
    }

    pub fn active_openai_profile_kind(&self) -> Option<OpenAiEndpointKind> {
        self.active_openai_profile().map(|profile| profile.kind)
    }

    pub fn select_openai_profile(
        &mut self,
        profile_id: impl Into<String>,
        label: impl Into<String>,
        kind: OpenAiEndpointKind,
    ) {
        self.sync_active_provider_state();
        self.provider = "openai-compatible".to_string();
        self.reset_provider_scoped_fields();

        let profile_id = profile_id.into();
        let label = label.into();
        let mut profile = self
            .openai_profiles
            .get(&profile_id)
            .cloned()
            .unwrap_or_else(|| self.default_openai_profile(&profile_id, label.as_str(), kind));
        profile.id = profile_id.clone();
        if profile.label.trim().is_empty() {
            profile.label = label;
        }
        profile.kind = kind;
        if profile
            .base_url
            .as_deref()
            .is_none_or(|value| value.trim().is_empty())
        {
            profile.base_url = Some(kind.default_base_url().to_string());
        }
        if profile
            .model
            .as_deref()
            .is_none_or(|value| value.trim().is_empty())
        {
            profile.model = Some(kind.default_model().to_string());
        }
        self.active_openai_profile_id = Some(profile_id.clone());
        self.openai_profiles.insert(profile_id, profile.clone());
        self.apply_openai_profile(profile);
    }

    fn sync_active_provider_state(&mut self) {
        if self.provider.trim().is_empty() {
            return;
        }
        if self.provider == "openai-compatible" {
            self.sync_active_openai_profile();
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

    fn apply_openai_profile(&mut self, profile: OpenAiEndpointProfile) {
        self.api_key = profile.api_key;
        self.base_url = profile.base_url;
        self.model = profile.model;
        self.reasoning_effort = profile.reasoning_effort;
        self.reasoning_summary = profile.reasoning_summary;
        self.revision = profile.revision;
        self.thinking = None;
        self.num_ctx = None;
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

    fn sync_active_openai_profile(&mut self) {
        let profile_id = self.ensure_active_openai_profile_id();
        let mut profile = self
            .openai_profiles
            .get(&profile_id)
            .cloned()
            .unwrap_or_else(|| {
                self.default_openai_profile(
                    &profile_id,
                    OpenAiEndpointKind::Custom.label(),
                    OpenAiEndpointKind::Custom,
                )
            });
        profile.id = profile_id.clone();
        profile.api_key = self.api_key.clone();
        profile.base_url = self.base_url.clone();
        profile.model = self.model.clone();
        profile.reasoning_effort = self.reasoning_effort.clone();
        profile.reasoning_summary = self.reasoning_summary.clone();
        profile.revision = self.revision.clone();
        self.openai_profiles.insert(profile_id, profile);
    }

    fn ensure_active_openai_profile_id(&mut self) -> String {
        if let Some(existing) = self.active_openai_profile_id() {
            return existing.to_string();
        }
        let id = OpenAiEndpointKind::Custom.default_profile_id().to_string();
        self.active_openai_profile_id = Some(id.clone());
        id
    }

    fn default_openai_profile(
        &self,
        profile_id: &str,
        label: &str,
        kind: OpenAiEndpointKind,
    ) -> OpenAiEndpointProfile {
        OpenAiEndpointProfile {
            id: profile_id.to_string(),
            label: label.to_string(),
            kind,
            api_key: None,
            base_url: Some(kind.default_base_url().to_string()),
            model: Some(kind.default_model().to_string()),
            reasoning_effort: None,
            reasoning_summary: Some(DEFAULT_REASONING_SUMMARY.to_string()),
            revision: None,
        }
    }

    fn profile_for_kind_or_default(&self, kind: OpenAiEndpointKind) -> OpenAiEndpointProfile {
        self.openai_profiles
            .get(kind.default_profile_id())
            .cloned()
            .unwrap_or_else(|| {
                self.default_openai_profile(kind.default_profile_id(), kind.label(), kind)
            })
    }

    fn migrate_legacy_openai_profiles(&mut self) {
        let mut migrated_profiles = BTreeMap::new();
        let mut active_profile_id = self.active_openai_profile_id.clone();
        let current_provider = self.provider.clone();
        let mut should_apply_active_profile = false;
        let mut should_switch_provider = false;

        for legacy_provider in ["openai-compatible", "deepseek", "kimi", "openrouter"] {
            let Some(kind) = OpenAiEndpointKind::from_legacy_provider(legacy_provider) else {
                continue;
            };
            let profile_id = kind.default_profile_id().to_string();
            let label = kind.label().to_string();

            if let Some(state) = self.provider_states.remove(legacy_provider) {
                migrated_profiles.insert(
                    profile_id.clone(),
                    OpenAiEndpointProfile {
                        id: profile_id.clone(),
                        label: label.clone(),
                        kind,
                        api_key: state.api_key,
                        base_url: normalize_optional_string(state.base_url)
                            .or_else(|| Some(kind.default_base_url().to_string())),
                        model: normalize_optional_string(state.model)
                            .or_else(|| Some(kind.default_model().to_string())),
                        reasoning_effort: normalize_optional_string(state.reasoning_effort),
                        reasoning_summary: normalize_reasoning_summary(state.reasoning_summary)
                            .or_else(|| Some(DEFAULT_REASONING_SUMMARY.to_string())),
                        revision: normalize_optional_string(state.revision),
                    },
                );
            }

            if current_provider == legacy_provider {
                should_apply_active_profile = true;
                should_switch_provider = legacy_provider != "openai-compatible";
                let existing_active_profile = if legacy_provider == "openai-compatible" {
                    self.active_openai_profile().cloned()
                } else {
                    None
                };
                if legacy_provider != "openai-compatible"
                    || (active_profile_id.is_none() && existing_active_profile.is_none())
                {
                    active_profile_id = Some(profile_id.clone());
                }
                let (target_profile_id, target_kind, target_label) =
                    if let Some(profile) = existing_active_profile {
                        (profile.id, profile.kind, profile.label)
                    } else {
                        (
                            active_profile_id
                                .clone()
                                .unwrap_or_else(|| profile_id.clone()),
                            kind,
                            label,
                        )
                    };
                migrated_profiles.insert(
                    target_profile_id.clone(),
                    OpenAiEndpointProfile {
                        id: target_profile_id,
                        label: target_label,
                        kind: target_kind,
                        api_key: self.api_key.clone(),
                        base_url: normalize_optional_string(self.base_url.clone()).or_else(|| {
                            Some(target_kind.default_base_url().to_string())
                        }),
                        model: normalize_optional_string(self.model.clone())
                            .or_else(|| Some(target_kind.default_model().to_string())),
                        reasoning_effort: normalize_optional_string(self.reasoning_effort.clone()),
                        reasoning_summary: normalize_reasoning_summary(
                            self.reasoning_summary.clone(),
                        )
                        .or_else(|| Some(DEFAULT_REASONING_SUMMARY.to_string())),
                        revision: normalize_optional_string(self.revision.clone()),
                    },
                );
            }
        }

        if !migrated_profiles.is_empty() {
            self.openai_profiles.extend(migrated_profiles);
        }
        if should_switch_provider {
            self.provider = "openai-compatible".to_string();
        }
        if should_apply_active_profile {
            self.active_openai_profile_id = active_profile_id;
            if let Some(profile) = self.active_openai_profile().cloned() {
                self.apply_openai_profile(profile);
            }
        }
    }
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

fn resolve_provider_value<'a>(
    provider_value: Option<&'a str>,
    legacy_value: Option<&'a str>,
    default_value: Option<&'a str>,
) -> ResolvedProviderValue<'a> {
    if let Some(value) = provider_value {
        return ResolvedProviderValue {
            value: Some(value),
            source: ConfigValueSource::ProviderState,
        };
    }
    if let Some(value) = legacy_value {
        return ResolvedProviderValue {
            value: Some(value),
            source: ConfigValueSource::LegacyGlobal,
        };
    }
    if let Some(value) = default_value {
        return ResolvedProviderValue {
            value: Some(value),
            source: ConfigValueSource::BuiltInDefault,
        };
    }
    ResolvedProviderValue {
        value: None,
        source: ConfigValueSource::Unset,
    }
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

#[cfg(test)]
mod tests {
    use super::{
        workspace_data_dir_for_home, ConfigManager, OpenAiEndpointKind, OpenAiEndpointProfile,
        ProviderConfigState, RaraConfig,
    };
    use crate::defaults::{
        DEFAULT_CODEX_BASE_URL, DEFAULT_CODEX_CHATGPT_BASE_URL, DEFAULT_CODEX_MODEL,
        DEFAULT_KIMI_BASE_URL, DEFAULT_KIMI_MODEL, DEFAULT_OPENROUTER_BASE_URL,
        DEFAULT_OPENROUTER_MODEL, DEFAULT_REASONING_SUMMARY, REASONING_SUMMARY_NONE,
    };
    use secrecy::ExposeSecret;
    use std::collections::BTreeMap;
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
            Some(DEFAULT_REASONING_SUMMARY)
        );
        assert_eq!(config.base_url.as_deref(), Some("http://localhost:11434"));
        assert_eq!(config.num_ctx, Some(32768));
    }

    #[test]
    fn migrate_legacy_openai_provider_into_active_profile() {
        let mut config = RaraConfig {
            provider: "kimi".to_string(),
            base_url: Some("https://api.moonshot.cn/v1".to_string()),
            model: Some("kimi-k2".to_string()),
            reasoning_summary: Some("detailed".to_string()),
            ..Default::default()
        };
        config.set_api_key("sk-kimi");

        config.migrate_legacy_provider_state();

        assert_eq!(config.provider, "openai-compatible");
        assert_eq!(config.active_openai_profile_id(), Some("kimi-default"));
        assert_eq!(
            config.active_openai_profile_kind(),
            Some(OpenAiEndpointKind::Kimi)
        );
        let profile = config
            .active_openai_profile()
            .expect("active openai profile");
        assert_eq!(profile.label, "Kimi");
        assert_eq!(
            profile.api_key.as_ref().map(|v| v.expose_secret()),
            Some("sk-kimi")
        );
        assert_eq!(
            profile.base_url.as_deref(),
            Some("https://api.moonshot.cn/v1")
        );
        assert_eq!(profile.model.as_deref(), Some("kimi-k2"));
        assert_eq!(config.model.as_deref(), Some("kimi-k2"));
    }

    #[test]
    fn provider_state_migration_preserves_multiple_openai_profiles() {
        let mut config = RaraConfig {
            provider: "openrouter".to_string(),
            provider_states: BTreeMap::from([
                (
                    "kimi".to_string(),
                    ProviderConfigState {
                        api_key: Some("sk-kimi".into()),
                        base_url: Some(DEFAULT_KIMI_BASE_URL.to_string()),
                        model: Some(DEFAULT_KIMI_MODEL.to_string()),
                        ..Default::default()
                    },
                ),
                (
                    "openrouter".to_string(),
                    ProviderConfigState {
                        api_key: Some("sk-openrouter".into()),
                        base_url: Some(DEFAULT_OPENROUTER_BASE_URL.to_string()),
                        model: Some(DEFAULT_OPENROUTER_MODEL.to_string()),
                        ..Default::default()
                    },
                ),
            ]),
            ..Default::default()
        };

        config.migrate_legacy_provider_state();

        assert_eq!(config.provider, "openai-compatible");
        assert_eq!(
            config.active_openai_profile_id(),
            Some("openrouter-default")
        );
        assert!(config.openai_profiles.contains_key("kimi-default"));
        assert!(config.openai_profiles.contains_key("openrouter-default"));
        assert!(config.provider_states.is_empty());
    }

    #[test]
    fn provider_state_migration_does_not_switch_unrelated_provider() {
        let mut config = RaraConfig {
            provider: "ollama".to_string(),
            model: Some("qwen3".to_string()),
            provider_states: BTreeMap::from([(
                "openrouter".to_string(),
                ProviderConfigState {
                    api_key: Some("sk-openrouter".into()),
                    base_url: Some(DEFAULT_OPENROUTER_BASE_URL.to_string()),
                    model: Some(DEFAULT_OPENROUTER_MODEL.to_string()),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };

        config.migrate_legacy_provider_state();

        assert_eq!(config.provider, "ollama");
        assert_eq!(config.model.as_deref(), Some("qwen3"));
        assert!(config.openai_profiles.contains_key("openrouter-default"));
    }

    #[test]
    fn provider_state_migration_preserves_existing_openai_active_profile_id() {
        let mut config = RaraConfig {
            provider: "openai-compatible".to_string(),
            active_openai_profile_id: Some("openrouter-main".to_string()),
            openai_profiles: BTreeMap::from([(
                "openrouter-main".to_string(),
                OpenAiEndpointProfile {
                    id: "openrouter-main".to_string(),
                    label: "OpenRouter main".to_string(),
                    kind: OpenAiEndpointKind::Openrouter,
                    ..Default::default()
                },
            )]),
            base_url: Some("https://openrouter.ai/api/v1".to_string()),
            model: Some("anthropic/claude-sonnet-4".to_string()),
            api_key: Some("sk-openrouter-main".into()),
            ..Default::default()
        };

        config.migrate_legacy_provider_state();

        assert_eq!(config.provider, "openai-compatible");
        assert_eq!(config.active_openai_profile_id(), Some("openrouter-main"));
        let profile = config
            .active_openai_profile()
            .expect("active openai profile");
        assert_eq!(profile.id, "openrouter-main");
        assert_eq!(profile.label, "OpenRouter main");
        assert_eq!(profile.kind, OpenAiEndpointKind::Openrouter);
        assert_eq!(profile.model.as_deref(), Some("anthropic/claude-sonnet-4"));
        assert_eq!(
            profile.api_key.as_ref().map(|value| value.expose_secret()),
            Some("sk-openrouter-main")
        );
    }

    #[test]
    fn switching_openai_profiles_restores_profile_specific_fields() {
        let mut config = RaraConfig::default();

        config.select_openai_profile(
            "openrouter-main",
            "OpenRouter main",
            OpenAiEndpointKind::Openrouter,
        );
        config.set_api_key("sk-openrouter-main");
        config.set_base_url(Some("https://openrouter.ai/api/v1".to_string()));
        config.set_model(Some("anthropic/claude-sonnet-4".to_string()));

        config.select_openai_profile(
            "openrouter-backup",
            "OpenRouter backup",
            OpenAiEndpointKind::Openrouter,
        );
        config.set_api_key("sk-openrouter-backup");
        config.set_model(Some("openai/gpt-4o-mini".to_string()));

        config.select_openai_profile(
            "openrouter-main",
            "OpenRouter main",
            OpenAiEndpointKind::Openrouter,
        );

        assert_eq!(config.provider, "openai-compatible");
        assert_eq!(config.active_openai_profile_id(), Some("openrouter-main"));
        assert_eq!(config.api_key(), Some("sk-openrouter-main"));
        assert_eq!(config.model.as_deref(), Some("anthropic/claude-sonnet-4"));
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

        assert_eq!(
            config.reasoning_summary.as_deref(),
            Some(REASONING_SUMMARY_NONE)
        );
        assert_eq!(
            config.provider_states["codex"].reasoning_summary.as_deref(),
            Some(DEFAULT_REASONING_SUMMARY)
        );
    }

    #[test]
    fn invalid_reasoning_summary_normalizes_to_auto() {
        let mut config = RaraConfig::default();
        config.set_reasoning_summary(Some("verbose".to_string()));

        assert_eq!(
            config.reasoning_summary.as_deref(),
            Some(DEFAULT_REASONING_SUMMARY)
        );
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

        assert_eq!(config.model.as_deref(), Some(DEFAULT_CODEX_MODEL));
        assert_eq!(config.base_url.as_deref(), Some(DEFAULT_CODEX_BASE_URL));
    }

    #[test]
    fn apply_codex_defaults_for_base_url_switches_between_known_codex_defaults() {
        let mut config = RaraConfig {
            provider: "codex".to_string(),
            base_url: Some(DEFAULT_CODEX_BASE_URL.to_string()),
            model: Some(DEFAULT_CODEX_MODEL.to_string()),
            ..Default::default()
        };

        config.apply_codex_defaults_for_base_url(DEFAULT_CODEX_CHATGPT_BASE_URL);

        assert_eq!(
            config.base_url.as_deref(),
            Some(DEFAULT_CODEX_CHATGPT_BASE_URL)
        );
        assert_eq!(config.model.as_deref(), Some(DEFAULT_CODEX_MODEL));
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
