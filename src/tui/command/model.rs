use crate::config::RaraConfig;

use super::super::state::{CommandSpec, TuiApp, PROVIDER_FAMILIES};
use super::{command_score, COMMAND_SPECS};

pub fn model_help_text(app: &TuiApp) -> String {
    let lines = PROVIDER_FAMILIES
        .iter()
        .enumerate()
        .map(|(provider_idx, (family, title, _))| {
            let provider_lines = if matches!(family, ProviderFamily::Codex) {
                if app.codex_model_options.is_empty() {
                    "  Sign in to load the current Codex model catalog.".to_string()
                } else {
                    app.codex_model_options
                        .iter()
                        .enumerate()
                        .map(|(idx, preset)| {
                            let marker = if app.config.provider == "codex"
                                && app.config.model.as_deref() == Some(preset.model.as_str())
                            {
                                "*"
                            } else {
                                " "
                            };
                            format!("{marker} {}. {} ({})", idx + 1, preset.label, preset.model)
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                }
            } else if matches!(family, ProviderFamily::DeepSeek) {
                app.deepseek_model_options
                    .iter()
                    .enumerate()
                    .map(|(idx, model)| {
                        let marker = if app.config.active_openai_profile_kind()
                            == Some(rara_config::OpenAiEndpointKind::Deepseek)
                            && app.config.model.as_deref() == Some(model.as_str())
                        {
                            "*"
                        } else {
                            " "
                        };
                        format!(
                            "{marker} {}. {model} (openai-compatible:deepseek/{model})",
                            idx + 1
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                current_model_presets(provider_idx)
                    .iter()
                    .enumerate()
                    .map(|(idx, (label, provider, model))| {
                        let marker = if app.config.provider == *provider
                            && app.config.model.as_deref() == Some(*model)
                        {
                            "*"
                        } else {
                            " "
                        };
                        let shortcut = match family {
                            ProviderFamily::Codex => (idx + 1).to_string(),
                            ProviderFamily::DeepSeek => (idx + 1).to_string(),
                            ProviderFamily::OpenAiCompatible => (idx + 1).to_string(),
                            ProviderFamily::CandleLocal => (idx + 1).to_string(),
                            ProviderFamily::Ollama => model.to_string(),
                        };
                        format!("{marker} {shortcut}. {label} ({provider}/{model})")
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            format!("{title}\n{provider_lines}")
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    format!(
        "Current model: {} / {}\n\nAvailable presets:\n{}\n\nGemma 4 Candle presets are marked experimental.\n\nUse /model to open the interactive provider and model flow.",
        app.config.provider,
        app.current_model_label(),
        lines
    )
}

pub fn api_key_status(config: &RaraConfig) -> &'static str {
    if !super::provider_requires_api_key(&config.provider) {
        "not-required"
    } else if config.has_api_key() {
        "configured"
    } else {
        "missing"
    }
}

pub fn is_local_provider(provider: &str) -> bool {
    matches!(
        provider,
        "local" | "local-candle" | "gemma4" | "qwen3" | "qwn3"
    )
}

#[cfg(test)]
pub fn normalize_command_token(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(|ch| ch.to_lowercase())
        .collect()
}

#[cfg(test)]
