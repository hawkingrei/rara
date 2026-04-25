use crate::config::{OpenAiEndpointKind, RaraConfig};

use super::ProviderFamily;

pub const CODEX_MODEL_PRESETS: [(&str, &str, &str); 0] = [];

pub const OPENAI_COMPATIBLE_MODEL_PRESETS: [(&str, &str, &str); 4] = [
    ("Custom endpoint", "openai-compatible", "gpt-4o-mini"),
    ("DeepSeek", "openai-compatible", "deepseek-chat"),
    ("Kimi", "openai-compatible", "moonshot-v1-8k"),
    ("OpenRouter", "openai-compatible", "openai/gpt-4o-mini"),
];

pub const LOCAL_MODEL_PRESETS: [(&str, &str, &str); 3] = [
    ("Gemma 4 E4B (Experimental)", "gemma4", "gemma4-e4b"),
    ("Gemma 4 E2B (Experimental)", "gemma4", "gemma4-e2b"),
    ("Qwn3 8B", "qwn3", "qwn3-8b"),
];

pub const OLLAMA_MODEL_PRESETS: [(&str, &str, &str); 3] = [
    ("Gemma 4", "ollama", "gemma4"),
    ("Gemma 4 E4B", "ollama", "gemma4:e4b"),
    ("Gemma 4 E2B", "ollama", "gemma4:e2b"),
];

pub fn selected_provider_family_idx_for_config(config: &RaraConfig) -> usize {
    match config.provider.as_str() {
        "codex" => 0,
        "openai-compatible" | "deepseek" | "kimi" | "openrouter" => 1,
        "ollama" | "ollama-native" | "ollama-openai" => 3,
        "gemma4" | "qwn3" | "qwen3" => 2,
        _ => 0,
    }
}

pub fn current_model_presets(
    provider_picker_idx: usize,
) -> &'static [(&'static str, &'static str, &'static str)] {
    match super::PROVIDER_FAMILIES[provider_picker_idx].0 {
        ProviderFamily::Codex => &CODEX_MODEL_PRESETS,
        ProviderFamily::OpenAiCompatible => &OPENAI_COMPATIBLE_MODEL_PRESETS,
        ProviderFamily::CandleLocal => &LOCAL_MODEL_PRESETS,
        ProviderFamily::Ollama => &OLLAMA_MODEL_PRESETS,
    }
}

pub fn selected_preset_idx_for_config(config: &RaraConfig, provider_picker_idx: usize) -> usize {
    if matches!(
        super::PROVIDER_FAMILIES[provider_picker_idx].0,
        ProviderFamily::OpenAiCompatible
    ) {
        let kind = config
            .active_openai_profile_kind()
            .unwrap_or(OpenAiEndpointKind::Custom);
        return openai_compatible_preset_index(kind);
    }
    current_model_presets(provider_picker_idx)
        .iter()
        .position(|(_, provider, model)| {
            config.provider == *provider && config.model.as_deref() == Some(*model)
        })
        .unwrap_or(0)
}

pub fn openai_compatible_preset_kind(idx: usize) -> OpenAiEndpointKind {
    match idx {
        1 => OpenAiEndpointKind::Deepseek,
        2 => OpenAiEndpointKind::Kimi,
        3 => OpenAiEndpointKind::Openrouter,
        _ => OpenAiEndpointKind::Custom,
    }
}

pub fn openai_compatible_preset_index(kind: OpenAiEndpointKind) -> usize {
    match kind {
        OpenAiEndpointKind::Custom => 0,
        OpenAiEndpointKind::Deepseek => 1,
        OpenAiEndpointKind::Kimi => 2,
        OpenAiEndpointKind::Openrouter => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        openai_compatible_preset_index, openai_compatible_preset_kind,
        selected_provider_family_idx_for_config,
    };
    use crate::config::{OpenAiEndpointKind, RaraConfig};

    #[test]
    fn keeps_generic_openai_compatible_provider_in_its_own_family() {
        let config = RaraConfig {
            provider: "openai-compatible".to_string(),
            ..RaraConfig::default()
        };

        assert_eq!(selected_provider_family_idx_for_config(&config), 1);
    }

    #[test]
    fn keeps_local_and_ollama_provider_families_stable() {
        let local = RaraConfig {
            provider: "gemma4".to_string(),
            ..RaraConfig::default()
        };
        let ollama = RaraConfig {
            provider: "ollama".to_string(),
            ..RaraConfig::default()
        };
        let ollama_native = RaraConfig {
            provider: "ollama-native".to_string(),
            ..RaraConfig::default()
        };
        let ollama_openai = RaraConfig {
            provider: "ollama-openai".to_string(),
            ..RaraConfig::default()
        };

        assert_eq!(selected_provider_family_idx_for_config(&local), 2);
        assert_eq!(selected_provider_family_idx_for_config(&ollama), 3);
        assert_eq!(selected_provider_family_idx_for_config(&ollama_native), 3);
        assert_eq!(selected_provider_family_idx_for_config(&ollama_openai), 3);
    }

    #[test]
    fn keeps_legacy_openai_endpoint_providers_in_openai_compatible_family() {
        for provider in ["deepseek", "kimi", "openrouter"] {
            let config = RaraConfig {
                provider: provider.to_string(),
                ..RaraConfig::default()
            };
            assert_eq!(selected_provider_family_idx_for_config(&config), 1);
        }
    }

    #[test]
    fn openai_preset_kind_roundtrips() {
        for kind in [
            OpenAiEndpointKind::Custom,
            OpenAiEndpointKind::Deepseek,
            OpenAiEndpointKind::Kimi,
            OpenAiEndpointKind::Openrouter,
        ] {
            assert_eq!(
                openai_compatible_preset_kind(openai_compatible_preset_index(kind)),
                kind
            );
        }
    }
}
