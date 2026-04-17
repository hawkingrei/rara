use crate::config::RaraConfig;

use super::ProviderFamily;

pub const CODEX_MODEL_PRESETS: [(&str, &str, &str); 2] = [
    ("Codex (OAuth)", "codex", "codex"),
    ("Codex (API Key)", "codex", "codex"),
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
        "ollama" => 2,
        _ => 1,
    }
}

pub fn current_model_presets(
    provider_picker_idx: usize,
) -> &'static [(&'static str, &'static str, &'static str)] {
    match super::PROVIDER_FAMILIES[provider_picker_idx].0 {
        ProviderFamily::Codex => &CODEX_MODEL_PRESETS,
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
