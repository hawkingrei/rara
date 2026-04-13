use crate::config::RaraConfig;
use std::fs;
use std::path::PathBuf;

use super::state::{
    current_model_presets, CommandSpec, LocalCommand, LocalCommandKind, ProviderFamily, TuiApp,
    PROVIDER_FAMILIES,
};

pub const COMMAND_SPECS: [CommandSpec; 8] = [
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
        usage: "/model",
        summary: "Open the guided provider and model switching flow.",
        detail: "Open the interactive model flow. First choose a provider family, then choose a concrete model. Ollama base URL editing lives inside that flow.",
    },
    CommandSpec {
        category: "Models",
        name: "base-url",
        usage: "/base-url",
        summary: "Open the provider base URL editor.",
        detail: "Open the interactive base URL editor. Use this mainly for Ollama. Edit and save the value inside the TUI instead of passing command arguments.",
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
        "base-url" => LocalCommandKind::BaseUrl,
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
        &["model", "base-url", "status", "help", "clear", "setup"]
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
    "RARA uses a single composer as the control surface.\n\nNormal input goes to the current agent.\nSlash commands stay local and open overlays or update runtime state.\n\nEditing:\n  apply_patch is the default tool for updating existing files\n  write_file is for new files or full rewrites\n  replace is only a simple fallback for unique string swaps\n\nKeyboard:\n  Enter submit current composer input\n  Esc close the current overlay only\n  Up/Down or j/k move inside lists\n  1/2/3 switch help tabs or choose guided model options\n\nExit:\n  /quit or /exit leave the TUI."
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
        "Built-in commands:\n{}\n\nEditing:\n  apply_patch  preferred for editing existing files\n  write_file   use for new files or full rewrites\n  replace      simple fallback for unique string replacement\n\nKeyboard:\n  Enter submit\n  Esc close current overlay\n  S open setup\n\nExit:\n  /quit\n  /exit\n\nModel switching:\n  /model\n\nProvider URL:\n  /base-url",
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
    let thinking = if app.config.provider == "ollama" || app.config.provider == "ollama-native" {
        if app.config.thinking.unwrap_or(true) {
            "on"
        } else {
            "off"
        }
    } else {
        "default"
    };
    let (device, dtype) = if is_local_provider(&app.config.provider) {
        crate::local_backend::local_runtime_target()
            .unwrap_or_else(|_| ("unavailable".to_string(), "unavailable".to_string()))
    } else {
        ("remote".to_string(), "-".to_string())
    };
    format!(
        "provider={}\nmodel={}\nbase_url={}\nrevision={}\nmode={}\napi_key={}\nthinking={}\ndevice={}\ndtype={}\nphase={}\ndetail={}",
        app.config.provider,
        app.current_model_label(),
        app.config.base_url.as_deref().unwrap_or("-"),
        app.config.revision.as_deref().unwrap_or("main"),
        mode,
        api_key_status(&app.config),
        thinking,
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

pub fn status_prompt_sources_text() -> String {
    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut sources = Vec::new();

    for name in ["AGENTS.md", "GEMINI.md", "CLAUDE.md"] {
        let path = root.join(name);
        if fs::metadata(&path).map(|meta| meta.is_file()).unwrap_or(false) {
            sources.push(format!("project instruction: {}", name));
        }
    }

    let rara_dir = root.join(".rara");
    let local_instructions = rara_dir.join("instructions.md");
    if fs::metadata(&local_instructions)
        .map(|meta| meta.is_file())
        .unwrap_or(false)
    {
        sources.push("local instruction: .rara/instructions.md".to_string());
    }

    let memory = rara_dir.join("memory.md");
    if fs::metadata(&memory)
        .map(|meta| meta.is_file())
        .unwrap_or(false)
    {
        sources.push("local memory: .rara/memory.md".to_string());
    }

    if sources.is_empty() {
        "No prompt sources discovered.".to_string()
    } else {
        sources.join("\n")
    }
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
     /base-url  open the provider URL editor\n\
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
        "Current model: {} / {}\n\nAvailable presets:\n{}\n\nGemma 4 Candle presets are marked experimental.\n\nUse /model to open the interactive provider and model flow.",
        app.config.provider,
        app.current_model_label(),
        lines
    )
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

#[cfg(test)]
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
        let command = parse_local_command("/model anything").expect("command should parse");
        assert!(matches!(command.kind, LocalCommandKind::Model));
        assert_eq!(command.arg.as_deref(), Some("anything"));
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
    fn parses_base_url_command() {
        let command = parse_local_command("/base-url").expect("command should parse");
        assert!(matches!(command.kind, LocalCommandKind::BaseUrl));
        assert_eq!(command.arg.as_deref(), None);
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
