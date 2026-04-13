use crate::config::RaraConfig;

use super::state::{
    current_model_presets, CommandSpec, LocalCommand, LocalCommandKind, ProviderFamily, TuiApp,
    LOCAL_MODEL_PRESETS, OLLAMA_MODEL_PRESETS, PROVIDER_FAMILIES,
};

pub const COMMAND_SPECS: [CommandSpec; 7] = [
    CommandSpec {
        category: "Session",
        name: "help",
        usage: "/help",
        summary: "Show built-in commands and keyboard hints.",
        detail: "Open the help modal with general guidance, command references, and runtime details.",
    },
    CommandSpec {
        category: "Session",
        name: "status",
        usage: "/status",
        summary: "Show current provider, model, revision, workspace, and runtime counters.",
        detail: "Open a runtime status modal with provider, model, revision, workspace, session, token counters, and cache location.",
    },
    CommandSpec {
        category: "Session",
        name: "clear",
        usage: "/clear",
        summary: "Clear the visible transcript and keep the current backend.",
        detail: "Reset only the local transcript view. The current backend, session id, and active runtime remain unchanged.",
    },
    CommandSpec {
        category: "Setup",
        name: "setup",
        usage: "/setup",
        summary: "Open the fallback setup screen.",
        detail: "Open the setup overlay for local model presets and OAuth. This is the fallback config surface, not the primary interaction flow.",
    },
    CommandSpec {
        category: "Models",
        name: "model",
        usage: "/model [name|1|2|3|next|list]",
        summary: "Open the model guide or switch local model presets in place.",
        detail: "Use without args to open the guided model flow. Use an explicit preset name or index to switch immediately and rebuild the backend in background.",
    },
    CommandSpec {
        category: "Setup",
        name: "login",
        usage: "/login",
        summary: "Start OAuth login in the background.",
        detail: "Start OAuth without blocking the TUI. Progress and completion are written back into the transcript and notice line.",
    },
    CommandSpec {
        category: "Session",
        name: "quit",
        usage: "/quit",
        summary: "Exit the TUI session.",
        detail: "Leave the RARA TUI and restore the terminal. The /exit alias behaves the same way.",
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
        "quit" | "exit" => LocalCommandKind::Quit,
        _ => return None,
    };

    Some(LocalCommand { kind, arg })
}

pub fn matching_commands(query: &str) -> Vec<&'static CommandSpec> {
    let query = query.trim();
    let mut matches = COMMAND_SPECS
        .iter()
        .filter_map(|spec| command_score(spec, query).map(|score| (score, spec)))
        .collect::<Vec<_>>();
    if matches.is_empty() {
        return COMMAND_SPECS.iter().collect();
    }
    matches.sort_by_key(|(score, spec)| (*score, spec.name));
    matches.into_iter().map(|(_, spec)| spec).collect()
}

pub fn command_spec_by_index(query: &str, index: usize) -> Option<&'static CommandSpec> {
    matching_commands(query).get(index).copied()
}

pub fn command_spec_by_name(name: &str) -> Option<&'static CommandSpec> {
    COMMAND_SPECS.iter().find(|spec| spec.name == name)
}

pub fn recommended_commands(app: &TuiApp) -> Vec<&'static CommandSpec> {
    let names: &[&str] = if app.is_busy() {
        &["status", "help", "clear"]
    } else {
        &["model", "status", "help", "clear", "setup"]
    };
    names
        .iter()
        .filter_map(|name| command_spec_by_name(name))
        .collect()
}

pub fn recent_command_specs(app: &TuiApp) -> Vec<&'static CommandSpec> {
    app.recent_commands
        .iter()
        .filter_map(|name| command_spec_by_name(name))
        .collect()
}

pub fn palette_commands(app: &TuiApp, query: &str) -> Vec<&'static CommandSpec> {
    if !query.trim().is_empty() {
        return matching_commands(query);
    }

    let mut commands = recommended_commands(app);
    for spec in recent_command_specs(app) {
        if !commands.iter().any(|existing| existing.name == spec.name) {
            commands.push(spec);
        }
    }
    if commands.is_empty() {
        COMMAND_SPECS.iter().collect()
    } else {
        commands
    }
}

pub fn palette_command_by_index(
    app: &TuiApp,
    query: &str,
    index: usize,
) -> Option<&'static CommandSpec> {
    palette_commands(app, query).get(index).copied()
}

pub fn command_detail_text(spec: &CommandSpec) -> String {
    format!("{}\n\n{}\n\n{}", spec.usage, spec.summary, spec.detail)
}

pub fn general_help_text() -> &'static str {
    "RARA uses a single composer as the control surface.\n\nNormal input goes to the current agent.\nSlash commands stay local and open overlays or update runtime state.\n\nKeyboard:\n  Enter submit current composer input\n  Esc close the current overlay only\n  Up/Down or j/k move inside lists\n  1/2/3 switch help tabs or choose guided model options\n\nExit:\n  /quit or /exit leave the TUI."
}

fn command_score(spec: &CommandSpec, query: &str) -> Option<u8> {
    if query.is_empty() {
        return Some(0);
    }
    let query = query.to_ascii_lowercase();
    let name = spec.name.to_ascii_lowercase();
    let usage = spec.usage.to_ascii_lowercase();
    let summary = spec.summary.to_ascii_lowercase();

    if name == query {
        Some(0)
    } else if name.starts_with(&query) {
        Some(1)
    } else if usage.contains(&query) {
        Some(2)
    } else if summary.contains(&query) {
        Some(3)
    } else {
        subsequence_match(&name, &query).then_some(4)
    }
}

fn subsequence_match(haystack: &str, needle: &str) -> bool {
    let mut chars = needle.chars();
    let mut current = chars.next();
    for ch in haystack.chars() {
        if Some(ch) == current {
            current = chars.next();
            if current.is_none() {
                return true;
            }
        }
    }
    current.is_none()
}

pub fn help_text() -> String {
    let commands = COMMAND_SPECS
        .iter()
        .map(|spec| format!("  {}  {}", spec.usage, spec.summary))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "Built-in commands:\n{}\n\nKeyboard:\n  Enter submit\n  Esc close current overlay\n  S open setup\n\nExit:\n  /quit\n  /exit\n\nModel switching examples:\n  /model\n  /model list\n  /model qwen3-8b\n  /model gemma4:e4b\n  /model next",
        commands
    )
}

pub fn status_runtime_text(app: &TuiApp) -> String {
    let mode = if super::provider_requires_api_key(&app.config.provider) {
        "hosted"
    } else {
        "local"
    };
    let phase = app.runtime_phase_label();
    let detail = app.runtime_phase_detail.as_deref().unwrap_or("-");
    let (device, dtype) = if is_local_provider(&app.config.provider) {
        crate::local_backend::local_runtime_target()
            .unwrap_or_else(|_| ("unavailable".to_string(), "unavailable".to_string()))
    } else {
        ("remote".to_string(), "-".to_string())
    };
    format!(
        "provider={}\nmodel={}\nrevision={}\nmode={}\napi_key={}\ndevice={}\ndtype={}\nphase={}\ndetail={}",
        app.config.provider,
        app.current_model_label(),
        app.config.revision.as_deref().unwrap_or("main"),
        mode,
        api_key_status(&app.config),
        device,
        dtype,
        phase,
        detail,
    )
}

pub fn status_workspace_text(app: &TuiApp) -> String {
    format!(
        "workspace={}\nbranch={}\nsession={}\nmessages={}\ntranscript={}",
        app.snapshot.cwd,
        app.snapshot.branch,
        app.snapshot.session_id,
        app.snapshot.history_len,
        app.transcript.len(),
    )
}

pub fn status_resources_text(app: &TuiApp) -> String {
    let cache = if is_local_provider(&app.config.provider) {
        crate::local_backend::default_local_model_cache_dir()
            .display()
            .to_string()
    } else {
        "-".to_string()
    };
    format!(
        "tokens={} in / {} out\ncache={}",
        app.snapshot.total_input_tokens,
        app.snapshot.total_output_tokens,
        cache,
    )
}

pub fn download_status_text(app: &TuiApp) -> Option<String> {
    if !matches!(
        app.runtime_phase,
        super::state::RuntimePhase::RebuildingBackend | super::state::RuntimePhase::BackendReady
    ) {
        return None;
    }

    let cache = if is_local_provider(&app.config.provider) {
        crate::local_backend::default_local_model_cache_dir()
            .display()
            .to_string()
    } else {
        "-".to_string()
    };
    let stage = app.runtime_phase_detail.as_deref().unwrap_or("waiting");
    let current_stage = infer_download_stage(stage);
    let steps = [
        ("setup", "Prepare request"),
        ("cache", "Resolve cache"),
        ("manifest", "Resolve manifest"),
        ("artifact", "Fetch tokenizer/config"),
        ("weights", "Fetch weights"),
        ("runtime", "Initialize runtime"),
        ("ready", "Model ready"),
    ]
    .into_iter()
    .enumerate()
    .map(|(idx, (key, label))| {
        let marker = if key == current_stage {
            ">"
        } else if download_stage_index(key) < download_stage_index(current_stage) {
            "x"
        } else {
            " "
        };
        format!("[{marker}] {}. {label}", idx + 1)
    })
    .collect::<Vec<_>>()
    .join("\n");
    Some(format!(
        "model={}\ncurrent={}\ncache={}\n\nsteps:\n{}",
        app.current_model_label(),
        stage,
        cache,
        steps,
    ))
}

fn infer_download_stage(detail: &str) -> &'static str {
    if detail.starts_with("Ready") {
        "ready"
    } else if detail.starts_with("Runtime") {
        "runtime"
    } else if detail.starts_with("Weights") {
        "weights"
    } else if detail.starts_with("Artifact") {
        "artifact"
    } else if detail.starts_with("Manifest") {
        "manifest"
    } else if detail.starts_with("Cache") {
        "cache"
    } else {
        "setup"
    }
}

fn download_stage_index(stage: &str) -> usize {
    match stage {
        "setup" => 0,
        "cache" => 1,
        "manifest" => 2,
        "artifact" => 3,
        "weights" => 4,
        "runtime" => 5,
        "ready" => 6,
        _ => 0,
    }
}

pub fn quick_actions_text() -> &'static str {
    "/help      browse commands and keyboard hints\n\
     /model     open guided model switching\n\
     /status    inspect runtime and workspace\n\
     /clear     reset the visible transcript\n\
     /setup     open fallback setup\n\
     /quit      leave the TUI"
}

pub fn recent_transcript_preview(app: &TuiApp, limit: usize) -> String {
    if app.transcript.is_empty() {
        return "No transcript entries yet.".to_string();
    }
    app.transcript
        .iter()
        .rev()
        .take(limit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|(role, message)| {
            let first_line = message.lines().next().unwrap_or("").trim();
            let preview = if first_line.is_empty() {
                "(empty)"
            } else {
                first_line
            };
            format!("{role}: {preview}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn model_help_text(app: &TuiApp) -> String {
    let lines = PROVIDER_FAMILIES
        .iter()
        .enumerate()
        .map(|(provider_idx, (family, title, _))| {
            let provider_lines = current_model_presets(provider_idx)
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
                        ProviderFamily::CandleLocal => (idx + 1).to_string(),
                        ProviderFamily::Ollama => model.to_string(),
                    };
                    format!("{marker} {shortcut}. {label} ({provider}/{model})")
                })
                .collect::<Vec<_>>()
                .join("\n");
            format!("{title}\n{provider_lines}")
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    format!(
        "Current model: {} / {}\n\nAvailable presets:\n{}\n\nGemma 4 Candle presets are marked experimental.\n\nUse /model to open the guide, /model list to inspect providers, /model <name> to switch directly, or /model next to rotate within the current provider family.",
        app.config.provider,
        app.current_model_label(),
        lines
    )
}

pub fn resolve_model_selection(arg: &str, app: &TuiApp) -> Option<(usize, usize)> {
    match arg {
        "1" => Some((app.provider_picker_idx, 0)),
        "2" => Some((app.provider_picker_idx, 1)),
        "3" => Some((app.provider_picker_idx, 2)),
        "next" => Some((
            app.provider_picker_idx,
            (app.selected_preset_idx() + 1) % current_model_presets(app.provider_picker_idx).len(),
        )),
        _ => current_model_presets(app.provider_picker_idx)
            .iter()
            .position(|(label, provider, model)| {
                *model == arg
                    || *provider == arg
                    || normalize_command_token(label) == normalize_command_token(arg)
                    || (*model == "qwn3-8b" && arg.eq_ignore_ascii_case("qwen3-8b"))
                    || (*model == "gemma4-e4b" && arg.eq_ignore_ascii_case("gemma-4-e4b"))
                    || (*model == "gemma4-e2b" && arg.eq_ignore_ascii_case("gemma-4-e2b"))
            })
            .map(|idx| (app.provider_picker_idx, idx))
            .or_else(|| {
                OLLAMA_MODEL_PRESETS.iter().position(|(label, provider, model)| {
                    *model == arg
                        || *provider == arg
                        || normalize_command_token(label) == normalize_command_token(arg)
                }).map(|idx| (1, idx))
            })
            .or_else(|| {
                LOCAL_MODEL_PRESETS.iter().position(|(label, provider, model)| {
                    *model == arg
                        || *provider == arg
                        || normalize_command_token(label) == normalize_command_token(arg)
                        || (*model == "qwn3-8b" && arg.eq_ignore_ascii_case("qwen3-8b"))
                        || (*model == "gemma4-e4b" && arg.eq_ignore_ascii_case("gemma-4-e4b"))
                        || (*model == "gemma4-e2b" && arg.eq_ignore_ascii_case("gemma-4-e2b"))
                }).map(|idx| (0, idx))
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
    fn parses_quit_aliases() {
        let quit = parse_local_command("/quit").expect("quit should parse");
        assert!(matches!(quit.kind, LocalCommandKind::Quit));

        let exit = parse_local_command("/exit").expect("exit should parse");
        assert!(matches!(exit.kind, LocalCommandKind::Quit));
    }

    #[test]
    fn matches_commands_by_prefix() {
        let names = matching_commands("st")
            .into_iter()
            .map(|spec| spec.name)
            .collect::<Vec<_>>();
        assert_eq!(names.first().copied(), Some("status"));
        assert!(names.contains(&"setup"));
    }

    #[test]
    fn normalizes_model_labels_for_command_matching() {
        assert_eq!(normalize_command_token("Gemma 4 E4B"), "gemma4e4b");
        assert_eq!(normalize_command_token("Qwn3 8B"), "qwn38b");
    }

    #[test]
    fn exact_and_prefix_matches_rank_ahead_of_fuzzy_matches() {
        let names = matching_commands("model")
            .into_iter()
            .map(|spec| spec.name)
            .collect::<Vec<_>>();
        assert_eq!(names.first().copied(), Some("model"));
    }
}
