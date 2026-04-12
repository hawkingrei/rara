use crate::config::RaraConfig;

use super::state::{CommandSpec, LocalCommand, LocalCommandKind, TuiApp, LOCAL_MODEL_PRESETS};

pub const COMMAND_SPECS: [CommandSpec; 6] = [
    CommandSpec {
        name: "help",
        usage: "/help",
        summary: "Show built-in commands and keyboard hints.",
    },
    CommandSpec {
        name: "status",
        usage: "/status",
        summary: "Show current provider, model, revision, workspace, and runtime counters.",
    },
    CommandSpec {
        name: "clear",
        usage: "/clear",
        summary: "Clear the visible transcript and keep the current backend.",
    },
    CommandSpec {
        name: "setup",
        usage: "/setup",
        summary: "Open the fallback setup screen.",
    },
    CommandSpec {
        name: "model",
        usage: "/model [name|1|2|3|next|list]",
        summary: "Open the picker or switch local model presets in place.",
    },
    CommandSpec {
        name: "login",
        usage: "/login",
        summary: "Start OAuth login in the background.",
    },
];

pub fn parse_local_command(input: &str) -> Option<LocalCommand> {
    let trimmed = input.trim();
    let command = trimmed.strip_prefix('/')?;
    let mut parts = command.splitn(2, char::is_whitespace);
    let name = parts.next()?.trim();
    let arg = parts
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    let kind = match name {
        "help" => LocalCommandKind::Help,
        "status" => LocalCommandKind::Status,
        "clear" => LocalCommandKind::Clear,
        "setup" => LocalCommandKind::Setup,
        "model" => LocalCommandKind::Model,
        "login" => LocalCommandKind::Login,
        _ => return None,
    };

    Some(LocalCommand { kind, arg })
}

pub fn matching_commands(query: &str) -> Vec<&'static CommandSpec> {
    let query = query.trim();
    let mut matches = COMMAND_SPECS
        .iter()
        .filter(|spec| query.is_empty() || spec.name.starts_with(query))
        .collect::<Vec<_>>();
    if matches.is_empty() {
        matches = COMMAND_SPECS.iter().collect();
    }
    matches
}

pub fn help_text() -> String {
    let commands = COMMAND_SPECS
        .iter()
        .map(|spec| format!("  {}  {}", spec.usage, spec.summary))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "Built-in commands:\n{}\n\nKeyboard:\n  Enter submit\n  Esc quit or leave current panel\n  S open setup\n\nModel switching examples:\n  /model\n  /model list\n  /model 2\n  /model qwen3-8b\n  /model next",
        commands
    )
}

pub fn status_text(app: &TuiApp) -> String {
    let local_cache = if is_local_provider(&app.config.provider) {
        format!(
            "\ncache={}",
            crate::local_backend::default_local_model_cache_dir().display()
        )
    } else {
        String::new()
    };
    format!(
        "provider={}\nmodel={}\nrevision={}\nworkspace={}\nbranch={}\nsession={}\nmessages={}\ntranscript={}\napi_key={}\ntokens={} in / {} out{}",
        app.config.provider,
        app.current_model_label(),
        app.config.revision.as_deref().unwrap_or("main"),
        app.snapshot.cwd,
        app.snapshot.branch,
        app.snapshot.session_id,
        app.snapshot.history_len,
        app.transcript.len(),
        api_key_status(&app.config),
        app.snapshot.total_input_tokens,
        app.snapshot.total_output_tokens,
        local_cache,
    )
}

pub fn model_help_text(app: &TuiApp) -> String {
    let lines = LOCAL_MODEL_PRESETS
        .iter()
        .enumerate()
        .map(|(idx, (label, provider, model))| {
            let marker =
                if app.config.provider == *provider && app.config.model.as_deref() == Some(*model) {
                    "*"
                } else {
                    " "
                };
            format!("{marker} {}. {} ({}/{})", idx + 1, label, provider, model)
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "Current model: {} / {}\n\nAvailable presets:\n{}\n\nUse /model to open the picker, or /model <name>, /model <1|2|3>, /model list, /model next.",
        app.config.provider,
        app.current_model_label(),
        lines
    )
}

pub fn resolve_model_selection(arg: &str, app: &TuiApp) -> Option<usize> {
    match arg {
        "1" => Some(0),
        "2" => Some(1),
        "3" => Some(2),
        "next" => Some((app.selected_preset_idx() + 1) % LOCAL_MODEL_PRESETS.len()),
        _ => LOCAL_MODEL_PRESETS
            .iter()
            .position(|(label, provider, model)| {
                *model == arg
                    || *provider == arg
                    || normalize_command_token(label) == normalize_command_token(arg)
                    || (*model == "qwn3-8b" && arg.eq_ignore_ascii_case("qwen3-8b"))
                    || (*model == "gemma4-e4b" && arg.eq_ignore_ascii_case("gemma-4-e4b"))
                    || (*model == "gemma4-e2b" && arg.eq_ignore_ascii_case("gemma-4-e2b"))
            }),
    }
}

pub fn api_key_status(config: &RaraConfig) -> &'static str {
    if !super::provider_requires_api_key(&config.provider) {
        "not-required"
    } else if config.api_key.as_ref().is_some() {
        "configured"
    } else {
        "missing"
    }
}

pub fn is_local_provider(provider: &str) -> bool {
    matches!(provider, "local" | "local-candle" | "gemma4" | "qwen3" | "qwn3")
}

pub fn normalize_command_token(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(|ch| ch.to_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{matching_commands, normalize_command_token, parse_local_command, LocalCommandKind};

    #[test]
    fn parses_model_command_argument() {
        let command = parse_local_command("/model qwen3-8b").expect("command should parse");
        assert!(matches!(command.kind, LocalCommandKind::Model));
        assert_eq!(command.arg.as_deref(), Some("qwen3-8b"));
    }

    #[test]
    fn returns_none_for_unknown_command() {
        assert!(parse_local_command("/unknown").is_none());
    }

    #[test]
    fn matches_commands_by_prefix() {
        let names = matching_commands("st")
            .into_iter()
            .map(|spec| spec.name)
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["status"]);
    }

    #[test]
    fn normalizes_model_labels_for_command_matching() {
        assert_eq!(normalize_command_token("Gemma 4 E4B"), "gemma4e4b");
        assert_eq!(normalize_command_token("Qwn3 8B"), "qwn38b");
    }
}
