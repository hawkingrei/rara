use crate::config::RaraConfig;

use super::ProviderFamily;

pub const CODEX_MODEL_PRESETS: [(&str, &str, &str); 3] = [
    ("GPT-5 Codex", "codex", "gpt-5-codex"),
    ("GPT-5.1 Codex mini", "codex", "gpt-5.1-codex-mini"),
    ("GPT-5.1 Codex max", "codex", "gpt-5.1-codex-max"),
];

pub const OPENAI_COMPATIBLE_MODEL_PRESETS: [(&str, &str, &str); 1] =
    [("Custom endpoint", "openai-compatible", "gpt-4o-mini")];

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
        "openai-compatible" => 1,
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
    current_model_presets(provider_picker_idx)
        .iter()
        .position(|(_, provider, model)| {
            config.provider == *provider && config.model.as_deref() == Some(*model)
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::selected_provider_family_idx_for_config;
    use crate::config::RaraConfig;

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
}
