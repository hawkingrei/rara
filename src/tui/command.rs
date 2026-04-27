use crate::config::RaraConfig;

use super::state::{
    current_model_presets, CommandSpec, LocalCommand, LocalCommandKind, PendingInteractionSnapshot,
    ProviderFamily, TuiApp, PROVIDER_FAMILIES,
};

pub const COMMAND_SPECS: [CommandSpec; 18] = [
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
        name: "runtime",
        usage: "/runtime",
        summary: "Alias for /status.",
        detail: "Open the runtime status modal. This matches the runtime-focused naming used by other agent CLIs.",
    },
    CommandSpec {
        category: "Session",
        name: "context",
        usage: "/context",
        summary: "Inspect the effective runtime context for the current turn.",
        detail: "Open a context modal that explains the effective prompt sources, active sections, workspace/runtime state, plan state, compaction metadata, and pending interaction inputs for the current turn.",
    },
    CommandSpec {
        category: "Session",
        name: "memory",
        usage: "/memory",
        summary: "Alias for /context.",
        detail: "Open the context modal to inspect the effective assembled context, memory selection, and active runtime state.",
    },
    CommandSpec {
        category: "Session",
        name: "clear",
        usage: "/clear",
        summary: "Clear the visible transcript and keep the current backend.",
        detail: "Reset only the local transcript view. The current backend, session id, and active runtime remain unchanged.",
    },
    CommandSpec {
        category: "Session",
        name: "resume",
        usage: "/resume",
        summary: "Pick and restore a recent local thread.",
        detail: "Open the recent thread picker backed by the local thread store and rollout artifacts. This restores committed turns, plan state, and interaction cards for the selected thread.",
    },
    CommandSpec {
        category: "Session",
        name: "threads",
        usage: "/threads",
        summary: "Alias for /resume.",
        detail: "Open the recent thread picker and choose a local thread to restore.",
    },
    CommandSpec {
        category: "Session",
        name: "plan",
        usage: "/plan",
        summary: "Enter planning mode for the current task.",
        detail: "Switch the agent into read-only planning mode. In planning mode, inspection tools stay available, but editing, shell execution, memory writes, and sub-agent launch tools are hidden and blocked. RARA can inspect the codebase, clarify constraints, refine the implementation approach, and only stop for approval once a concrete plan is ready. For non-trivial tasks, RARA may also suggest entering planning mode before it starts execution.",
    },
    CommandSpec {
        category: "Session",
        name: "approval",
        usage: "/approval",
        summary: "Toggle bash approval between suggestion and always.",
        detail: "Toggle bash execution between suggestion-only mode and always-run mode. Suggestion mode keeps bash inside the plan/approval flow instead of executing immediately.",
    },
    CommandSpec {
        category: "Session",
        name: "compact",
        usage: "/compact",
        summary: "Compact the current conversation history immediately.",
        detail: "Force one explicit history compaction pass using the current backend summarizer. This keeps the current turn/session but replaces older history with a summary, closer to Codex manual compaction than a silent trim.",
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
        summary: "Open the provider auth picker.",
        detail: "Open the auth-mode picker for the active provider. For codex, this includes browser login, device-code login, and API-key auth.",
    },
    CommandSpec {
        category: "Setup",
        name: "auth",
        usage: "/auth",
        summary: "Alias for /login.",
        detail: "Open the provider auth picker for the active provider.",
    },
    CommandSpec {
        category: "Setup",
        name: "logout",
        usage: "/logout",
        summary: "Clear the saved provider credential.",
        detail: "Clear the saved access token or API key from local config for the active provider. If the active provider is codex, RARA rebuilds the backend so the running session no longer uses the old credential.",
    },
    CommandSpec {
        category: "Setup",
        name: "models",
        usage: "/models",
        summary: "Alias for /model.",
        detail: "Open the guided provider and model switching flow.",
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
        "runtime" => LocalCommandKind::Status,
        "context" => LocalCommandKind::Context,
        "memory" => LocalCommandKind::Context,
        "clear" => LocalCommandKind::Clear,
        "resume" => LocalCommandKind::Resume,
        "threads" => LocalCommandKind::Resume,
        "plan" => LocalCommandKind::Plan,
        "approval" => LocalCommandKind::Approval,
        "compact" => LocalCommandKind::Compact,
        "model" | "models" => LocalCommandKind::Model,
        "base-url" => LocalCommandKind::BaseUrl,
        "login" | "auth" => LocalCommandKind::Login,
        "logout" => LocalCommandKind::Logout,
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
        &["context", "help", "status"]
    } else {
        &["context", "help", "model", "resume", "status"]
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

pub fn palette_commands(_app: &TuiApp, query: &str) -> Vec<&'static CommandSpec> {
    if !query.trim().is_empty() {
        return matching_commands(query);
    }

    let mut commands = COMMAND_SPECS.iter().collect::<Vec<_>>();
    commands.sort_by_key(|spec| spec.name);
    commands
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
    "RARA uses a single composer as the control surface.\n\nNormal input goes to the current agent.\nSlash commands stay local and open overlays or update runtime state.\n\nCompaction:\n  /compact forces one history compaction pass\n\nContext:\n  /context shows the effective runtime context for the current turn\n\nModes:\n  /plan enters planning mode for the current task\n  RARA may suggest planning mode first for non-trivial repository work\n  /approval toggles bash approval between suggestion and always\n\nAuth:\n  /login opens the provider auth picker\n  /logout clears the saved provider credential\n\nEditing:\n  apply_patch is the default tool for updating existing files\n  write_file is for new files or full rewrites\n  replace is only a simple fallback for unique string swaps\n\nKeyboard:\n  Enter submit current composer input\n  Shift+Enter insert a newline in the composer\n  Esc close the current overlay only\n  Up/Down or j/k move inside lists\n  1/2/3 switch help tabs or choose guided model options\n\nExit:\n  /quit or /exit leave the TUI."
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
    let mut specs = COMMAND_SPECS.iter().collect::<Vec<_>>();
    specs.sort_by_key(|spec| spec.name);
    let commands = specs
        .into_iter()
        .map(|spec| format!("  {}  {}", spec.usage, spec.summary))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "Built-in commands:\n{}\n\nCompaction:\n  /compact   summarize older conversation history now\n\nThreads:\n  /resume    reopen a recent local thread\n\nModes:\n  /plan      enter planning mode for the current task\n  RARA may suggest planning mode for non-trivial tasks\n  /approval  toggle bash approval mode\n\nAuth:\n  /login     open the provider auth picker\n  /logout    clear the saved provider credential\n\nEditing:\n  apply_patch  preferred for editing existing files\n  write_file   use for new files or full rewrites\n  replace      simple fallback for unique string replacement\n\nKeyboard:\n  Enter submit\n  Shift+Enter insert newline\n  Esc close current overlay\n\nExit:\n  /quit\n  /exit\n\nModel switching:\n  /model\n\nProvider URL:\n  /base-url",
        commands
    )
}

fn format_pending_interaction(snapshot: &PendingInteractionSnapshot) -> String {
    let kind = match snapshot.kind {
        super::state::InteractionKind::RequestInput => "request_input",
        super::state::InteractionKind::Approval => "approval",
        super::state::InteractionKind::PlanApproval => "plan_approval",
    };
    let mut lines = vec![format!(
        "- kind={kind} title={} summary={}",
        snapshot.title,
        if snapshot.summary.is_empty() {
            "-"
        } else {
            snapshot.summary.as_str()
        }
    )];
    if !snapshot.options.is_empty() {
        lines.push(format!("  options={}", snapshot.options.len()));
    }
    if let Some(source) = snapshot.source.as_deref() {
        lines.push(format!("  source={source}"));
    }
    if let Some(note) = snapshot.note.as_deref() {
        lines.push(format!("  note={note}"));
    }
    lines.join("\n")
}

fn render_context_assembly_entries(app: &TuiApp, layer: &str, title: &str) -> String {
    let entries = app
        .snapshot
        .assembly_entries
        .iter()
        .filter(|entry| entry.layer == layer)
        .collect::<Vec<_>>();
    if entries.is_empty() {
        return format!("{title}\n  - None.");
    }

    let body = entries
        .into_iter()
        .map(|entry| {
            let path = entry
                .source_path
                .as_deref()
                .filter(|value| !value.is_empty())
                .unwrap_or("-");
            let injected = if entry.injected { "yes" } else { "no" };
            let budget = entry
                .budget_impact_tokens
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string());
            let dropped = entry.dropped_reason.as_deref().unwrap_or("-");
            format!(
                "  {}. {} ({})\n     path: {}\n     injected: {}\n     budget impact: {}\n     why: {}\n     dropped: {}",
                entry.order,
                entry.label,
                entry.kind,
                path,
                injected,
                budget,
                entry.inclusion_reason,
                dropped,
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!("{title}\n{body}")
}

fn render_memory_selection(app: &TuiApp) -> String {
    let budget = app
        .snapshot
        .memory_selection
        .selection_budget_tokens
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string());
    let selected = if app.snapshot.memory_selection.selected_items.is_empty() {
        "  - None.".to_string()
    } else {
        app.snapshot
            .memory_selection
            .selected_items
            .iter()
            .map(|item| {
                let budget = item
                    .budget_impact_tokens
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_string());
                format!(
                    "  {}. {} ({})\n     detail: {}\n     budget impact: {}\n     why selected: {}",
                    item.order, item.label, item.kind, item.detail, budget, item.selection_reason
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let available = if app.snapshot.memory_selection.available_items.is_empty() {
        "  - None.".to_string()
    } else {
        app.snapshot
            .memory_selection
            .available_items
            .iter()
            .map(|item| {
                format!(
                    "  {}. {} ({})\n     detail: {}\n     why available: {}\n     not injected: {}",
                    item.order,
                    item.label,
                    item.kind,
                    item.detail,
                    item.selection_reason,
                    item.dropped_reason.as_deref().unwrap_or("-")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let dropped = if app.snapshot.memory_selection.dropped_items.is_empty() {
        "  - None.".to_string()
    } else {
        app.snapshot
            .memory_selection
            .dropped_items
            .iter()
            .map(|item| {
                format!(
                    "  {}. {} ({})\n     detail: {}\n     why considered: {}\n     dropped: {}",
                    item.order,
                    item.label,
                    item.kind,
                    item.detail,
                    item.selection_reason,
                    item.dropped_reason.as_deref().unwrap_or("-")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        "Memory Selection\n  selection budget: {}\n  selected now:\n{}\n  available but not injected:\n{}\n  dropped by ranking or budget:\n{}",
        budget, selected, available, dropped
    )
}

pub fn status_context_text(app: &TuiApp) -> String {
    let prompt_warnings = if app.snapshot.prompt_warnings.is_empty() {
        None
    } else {
        Some(format!(
            "Warnings:\n{}",
            app.snapshot
                .prompt_warnings
                .iter()
                .map(|warning| format!("  - {warning}"))
                .collect::<Vec<_>>()
                .join("\n")
        ))
    };
    let plan_lines = if app.snapshot.plan_steps.is_empty() {
        "  - No active plan steps.".to_string()
    } else {
        app.snapshot
            .plan_steps
            .iter()
            .map(|(status, step)| format!("  - [{status}] {step}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let pending_interactions = if app.snapshot.pending_interactions.is_empty() {
        "  - None.".to_string()
    } else {
        app.snapshot
            .pending_interactions
            .iter()
            .map(format_pending_interaction)
            .map(|entry| format!("  {entry}"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let last_boundary = if let Some(version) = app.snapshot.last_compaction_boundary_version {
        let before = app
            .snapshot
            .last_compaction_boundary_before_tokens
            .map(|tokens| tokens.to_string())
            .unwrap_or_else(|| "-".to_string());
        let files = app
            .snapshot
            .last_compaction_boundary_recent_file_count
            .map(|count| count.to_string())
            .unwrap_or_else(|| "-".to_string());
        format!("v{version} before_tokens={before} recent_file_count={files}")
    } else {
        "-".to_string()
    };
    let mut sections = vec![
        format!(
            "Current Session\n  cwd: {}\n  branch: {}\n  session: {}\n  history messages: {}\n  transcript entries: {}",
            app.snapshot.cwd,
            app.snapshot.branch,
            app.snapshot.session_id,
            app.snapshot.history_len,
            app.transcript_entry_count(),
        ),
        format!(
            "Context Budget\n  context window: {}\n  reserved output: {}\n  stable instructions: {}\n  workspace prompt sources: {}\n  active turn: {}\n  compacted history: {}\n  retrieved memory: {}\n  remaining input budget: {}",
            app.snapshot
                .context_window_tokens
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string()),
            app.snapshot.reserved_output_tokens,
            app.snapshot.stable_instructions_budget,
            app.snapshot.workspace_prompt_budget,
            app.snapshot.active_turn_budget,
            app.snapshot.compacted_history_budget,
            app.snapshot.retrieved_memory_budget,
            app.snapshot
                .remaining_input_budget
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string()),
        ),
        render_context_assembly_entries(app, "stable_instructions", "Stable Instructions"),
        render_context_assembly_entries(app, "workspace_prompt_sources", "Workspace Prompt Sources"),
        render_context_assembly_entries(app, "active_memory_inputs", "Active Memory Inputs"),
        render_memory_selection(app),
        render_context_assembly_entries(app, "compacted_history", "Compacted History"),
        render_context_assembly_entries(app, "active_turn_state", "Active Turn State"),
        render_context_assembly_entries(
            app,
            "retrieval_ready",
            "Retrieval-ready but not injected items",
        ),
        format!(
            "Plan State\n  explanation: {}\n{}",
            app.snapshot.plan_explanation.as_deref().unwrap_or("-"),
            plan_lines
        ),
        format!(
            "Compaction State\n  estimated history tokens: {}\n  context window: {}\n  threshold: {}\n  reserved output: {}\n  compactions: {}\n  last compaction: {} -> {}\n  last boundary: {}",
            app.snapshot.estimated_history_tokens,
            app.snapshot
                .context_window_tokens
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string()),
            app.snapshot.compact_threshold_tokens,
            app.snapshot.reserved_output_tokens,
            app.snapshot.compaction_count,
            app.snapshot
                .last_compaction_before_tokens
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string()),
            app.snapshot
                .last_compaction_after_tokens
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string()),
            last_boundary
        ),
        format!("Pending Interaction\n{}", pending_interactions),
    ];
    if let Some(warnings) = prompt_warnings {
        sections.insert(2, warnings);
    }
    sections.join("\n\n")
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
        "-"
    };
    let surface = app.config.effective_provider_surface();
    let reasoning_summary = surface
        .reasoning_summary
        .display_or(rara_config::DEFAULT_REASONING_SUMMARY);
    let reasoning_effort_label = app.current_reasoning_effort_label();
    let endpoint_profile = if app.config.provider == "openai-compatible" {
        app.config.active_openai_profile_label().unwrap_or("-")
    } else {
        "-"
    };
    let endpoint_kind = if app.config.provider == "openai-compatible" {
        app.config
            .active_openai_profile_kind()
            .map(|kind| match kind {
                rara_config::OpenAiEndpointKind::Custom => "custom",
                rara_config::OpenAiEndpointKind::Deepseek => "deepseek",
                rara_config::OpenAiEndpointKind::Kimi => "kimi",
                rara_config::OpenAiEndpointKind::Openrouter => "openrouter",
            })
            .unwrap_or("-")
    } else {
        "-"
    };
    let codex_auth_mode = match app.codex_auth_mode {
        Some(crate::oauth::SavedCodexAuthMode::ApiKey) => "api_key",
        Some(crate::oauth::SavedCodexAuthMode::Chatgpt) => "chatgpt",
        None => "-",
    };
    let codex_endpoint_kind = if app.config.provider == "codex" {
        match app.codex_auth_mode {
            Some(crate::oauth::SavedCodexAuthMode::Chatgpt) => "chatgpt_codex",
            Some(crate::oauth::SavedCodexAuthMode::ApiKey) => "openai_api",
            None => "unknown",
        }
    } else {
        "-"
    };
    let (device, dtype) = if is_local_provider(&app.config.provider) {
        crate::local_backend::local_runtime_target()
            .unwrap_or_else(|_| ("unavailable".to_string(), "unavailable".to_string()))
    } else {
        ("remote".to_string(), "-".to_string())
    };
    format!(
        "provider={}\nendpoint_profile={}\nendpoint_kind={}\nmodel={}\nmodel_source={}\nbase_url={}\nbase_url_source={}\nrevision={}\nrevision_source={}\nagent_mode={}\nbash_approval={}\nmode={}\napi_key={}\napi_key_source={}\ncodex_auth_mode={}\ncodex_endpoint_kind={}\nthinking={}\nreasoning_summary={}\nreasoning_summary_source={}\nreasoning_effort={}\nreasoning_effort_source={}\ndevice={}\ndtype={}\nfocused={}\nphase={}\ndetail={}",
        surface.provider,
        endpoint_profile,
        endpoint_kind,
        surface.model.display_or(app.current_model_label()),
        surface.model.source.label(),
        surface.base_url.display_or("-"),
        surface.base_url.source.label(),
        surface.revision.display_or("main"),
        surface.revision.source.label(),
        app.agent_execution_mode_label(),
        app.bash_approval_mode_label(),
        mode,
        api_key_status(&app.config),
        surface.api_key.source.label(),
        codex_auth_mode,
        codex_endpoint_kind,
        thinking,
        reasoning_summary,
        surface.reasoning_summary.source.label(),
        surface.reasoning_effort.display_or(reasoning_effort_label.as_str()),
        surface.reasoning_effort.source.label(),
        device,
        dtype,
        app.terminal_focused,
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
        app.transcript_entry_count(),
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
    let state_db = app.state_db_status.as_deref().unwrap_or("-");
    let context = match app.snapshot.context_window_tokens {
        Some(window) => format!(
            "{} / {} (~auto @ {}, reserve {})",
            app.snapshot.estimated_history_tokens,
            window,
            app.snapshot.compact_threshold_tokens,
            app.snapshot.reserved_output_tokens
        ),
        None => format!(
            "{} (~auto @ {})",
            app.snapshot.estimated_history_tokens, app.snapshot.compact_threshold_tokens
        ),
    };
    let last_compact = match (
        app.snapshot.last_compaction_before_tokens,
        app.snapshot.last_compaction_after_tokens,
    ) {
        (Some(before), Some(after)) => format!("{before} -> {after}"),
        _ => "-".to_string(),
    };
    let recent_compact_files = if app.snapshot.last_compaction_recent_files.is_empty() {
        "-".to_string()
    } else {
        app.snapshot.last_compaction_recent_files.join(", ")
    };
    let recent_compact_file_count = app.snapshot.last_compaction_recent_files.len();
    let last_compact_ratio = match (
        app.snapshot.last_compaction_before_tokens,
        app.snapshot.last_compaction_after_tokens,
    ) {
        (Some(before), Some(after)) if before > 0 => format!("{:.2}", after as f64 / before as f64),
        _ => "-".to_string(),
    };
    let compact_boundary = match (
        app.snapshot.last_compaction_boundary_version,
        app.snapshot.last_compaction_boundary_before_tokens,
        app.snapshot.last_compaction_boundary_recent_file_count,
    ) {
        (Some(version), Some(before), Some(file_count)) => {
            format!("v{version} (before_tokens={before}, recent_file_count={file_count})")
        }
        _ => "-".to_string(),
    };
    let retrieval_budget = app
        .snapshot
        .memory_selection
        .selection_budget_tokens
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string());
    let retrieval_selected = if app.snapshot.memory_selection.selected_items.is_empty() {
        "-".to_string()
    } else {
        app.snapshot
            .memory_selection
            .selected_items
            .iter()
            .map(|item| item.kind.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "tokens={} in / {} out\ncontext_estimate={}\nretrieval_budget={}\nretrieval_selected={}\ncompactions={} (last: {}, ratio: {})\ncompact_boundary={}\nrecent_compact_file_count={}\nrecent_compact_files={}\ncache={}\nstate_db={}",
        app.snapshot.total_input_tokens,
        app.snapshot.total_output_tokens,
        context,
        retrieval_budget,
        retrieval_selected,
        app.snapshot.compaction_count,
        last_compact,
        last_compact_ratio,
        compact_boundary,
        recent_compact_file_count,
        recent_compact_files,
        cache,
        state_db,
    )
}

pub fn status_prompt_sources_text(app: &TuiApp) -> String {
    let mut lines = vec![
        format!("base prompt: {}", app.snapshot.prompt_base_kind),
        format!(
            "active sections: {}",
            app.snapshot.prompt_section_keys.join(", ")
        ),
        String::new(),
    ];
    if app.snapshot.prompt_source_entries.is_empty() {
        lines.push("no structured prompt sources discovered".to_string());
    } else {
        lines.extend(app.snapshot.prompt_source_entries.iter().map(|entry| {
            format!(
                "{}. {} [{}] {}\n   why: {}",
                entry.order, entry.label, entry.kind, entry.display_path, entry.inclusion_reason
            )
        }));
    }
    if !app.snapshot.prompt_warnings.is_empty() {
        if lines.len() > 3 {
            lines.push(String::new());
        }
        lines.push("warnings:".to_string());
        lines.extend(
            app.snapshot
                .prompt_warnings
                .iter()
                .map(|warning| format!("- {warning}")),
        );
    }
    if app.snapshot.prompt_source_entries.is_empty() && app.snapshot.prompt_warnings.is_empty() {
        "No prompt sources discovered.".to_string()
    } else {
        lines.join("\n")
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
    "/approval  toggle bash approval mode\n\
     /base-url  open the provider URL editor\n\
     /clear     reset the visible transcript\n\
     /context   inspect effective runtime context\n\
     /help      browse commands and keyboard hints\n\
     /model     open guided model switching\n\
     /plan      enter planning mode for the current task\n\
     /status    inspect runtime and workspace\n\
     /quit      leave the TUI"
}

pub fn recent_transcript_preview(app: &TuiApp, limit: usize) -> String {
    let mut entries = app
        .committed_turns
        .iter()
        .flat_map(|turn| turn.entries.iter())
        .chain(app.active_turn.entries.iter())
        .collect::<Vec<_>>();
    if entries.is_empty() {
        return "No transcript entries yet.".to_string();
    }
    entries
        .drain(..)
        .rev()
        .take(limit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|entry| {
            let first_line = entry.message.lines().next().unwrap_or("").trim();
            let preview = if first_line.is_empty() {
                "(empty)"
            } else {
                first_line
            };
            format!("{}: {preview}", entry.role)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn current_turn_preview(app: &TuiApp, limit: usize) -> String {
    if app.active_turn.entries.is_empty() {
        return "No active turn yet.".to_string();
    }

    app.active_turn
        .entries
        .iter()
        .take(limit)
        .map(|entry| {
            let first_line = entry.message.lines().next().unwrap_or("").trim();
            let preview = if first_line.is_empty() {
                "(empty)"
            } else {
                first_line
            };
            format!("{}: {preview}", entry.role)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

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
mod tests {
    use super::{
        help_text, matching_commands, model_help_text, normalize_command_token, palette_commands,
        parse_local_command, status_context_text, status_prompt_sources_text,
        status_resources_text, status_runtime_text, LocalCommandKind, COMMAND_SPECS,
    };
    use crate::config::{ConfigManager, OpenAiEndpointKind};
    use crate::context::PromptSourceContextEntry;
    use crate::tui::state::{RuntimeSnapshot, TuiApp};
    use tempfile::tempdir;

    #[test]
    fn parses_model_command_argument() {
        let command = parse_local_command("/model anything").expect("command should parse");
        assert!(matches!(command.kind, LocalCommandKind::Model));
        assert_eq!(command.arg.as_deref(), Some("anything"));
    }

    #[test]
    fn model_help_text_labels_deepseek_as_openai_compatible_endpoint() {
        let dir = tempdir().expect("tempdir");
        let mut app = TuiApp::new(ConfigManager {
            path: dir.path().join("config.json"),
        })
        .expect("app");
        app.config.select_openai_profile(
            "deepseek-default",
            "DeepSeek",
            OpenAiEndpointKind::Deepseek,
        );
        app.config.set_model(Some("deepseek-chat".to_string()));
        app.set_deepseek_model_options(vec!["deepseek-chat".to_string()]);

        let rendered = model_help_text(&app);

        assert!(rendered.contains("* 1. deepseek-chat (openai-compatible:deepseek/deepseek-chat)"));
        assert!(!rendered.contains("(deepseek/deepseek-chat)"));
    }

    #[test]
    fn parses_context_command() {
        let command = parse_local_command("/context").expect("command should parse");
        assert!(matches!(command.kind, LocalCommandKind::Context));
        assert!(command.arg.is_none());
    }

    #[test]
    fn parses_alias_commands() {
        let runtime = parse_local_command("/runtime").expect("runtime should parse");
        assert!(matches!(runtime.kind, LocalCommandKind::Status));

        let memory = parse_local_command("/memory").expect("memory should parse");
        assert!(matches!(memory.kind, LocalCommandKind::Context));

        let threads = parse_local_command("/threads").expect("threads should parse");
        assert!(matches!(threads.kind, LocalCommandKind::Resume));

        let auth = parse_local_command("/auth").expect("auth should parse");
        assert!(matches!(auth.kind, LocalCommandKind::Login));

        let models = parse_local_command("/models").expect("models should parse");
        assert!(matches!(models.kind, LocalCommandKind::Model));
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
    fn parses_resume_command() {
        let command = parse_local_command("/resume").expect("command should parse");
        assert!(matches!(command.kind, LocalCommandKind::Resume));
    }

    #[test]
    fn parses_plan_command() {
        let plan = parse_local_command("/plan").expect("plan should parse");
        assert!(matches!(plan.kind, LocalCommandKind::Plan));
    }

    #[test]
    fn parses_approval_command() {
        let approval = parse_local_command("/approval").expect("approval should parse");
        assert!(matches!(approval.kind, LocalCommandKind::Approval));
    }

    #[test]
    fn parses_compact_command() {
        let command = parse_local_command("/compact").expect("compact should parse");
        assert!(matches!(command.kind, LocalCommandKind::Compact));
        assert_eq!(command.arg.as_deref(), None);
    }

    #[test]
    fn parses_login_and_logout_commands() {
        let login = parse_local_command("/login").expect("login should parse");
        assert!(matches!(login.kind, LocalCommandKind::Login));
        assert_eq!(login.arg.as_deref(), None);

        let logout = parse_local_command("/logout").expect("logout should parse");
        assert!(matches!(logout.kind, LocalCommandKind::Logout));
        assert_eq!(logout.arg.as_deref(), None);
    }

    #[test]
    fn matches_commands_by_prefix() {
        let names = matching_commands("st")
            .into_iter()
            .map(|spec| spec.name)
            .collect::<Vec<_>>();
        assert_eq!(names.first().copied(), Some("status"));
        assert!(!names.contains(&"setup"));
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

    #[test]
    fn palette_commands_are_sorted_alphabetically_for_empty_query() {
        let dir = tempdir().expect("tempdir");
        let app = TuiApp::new(ConfigManager {
            path: dir.path().join("config.json"),
        })
        .expect("app");

        let names = palette_commands(&app, "")
            .into_iter()
            .map(|spec| spec.name)
            .collect::<Vec<_>>();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
    }

    #[test]
    fn palette_commands_show_full_command_list_for_empty_query() {
        let dir = tempdir().expect("tempdir");
        let app = TuiApp::new(ConfigManager {
            path: dir.path().join("config.json"),
        })
        .expect("app");

        let names = palette_commands(&app, "")
            .into_iter()
            .map(|spec| spec.name)
            .collect::<Vec<_>>();
        assert_eq!(names.len(), COMMAND_SPECS.len());
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
    }

    #[test]
    fn help_text_lists_built_in_commands_alphabetically() {
        let rendered = help_text();
        let approval_idx = rendered.find("/approval").expect("approval");
        let base_url_idx = rendered.find("/base-url").expect("base-url");
        let clear_idx = rendered.find("/clear").expect("clear");
        let compact_idx = rendered.find("/compact").expect("compact");
        let context_idx = rendered.find("/context").expect("context");

        assert!(approval_idx < base_url_idx);
        assert!(base_url_idx < clear_idx);
        assert!(clear_idx < compact_idx);
        assert!(compact_idx < context_idx);
    }

    #[test]
    fn status_resources_text_includes_recent_compacted_files() {
        let dir = tempdir().expect("tempdir");
        let cm = ConfigManager {
            path: dir.path().join("config.json"),
        };
        let mut app = TuiApp::new(cm).expect("app");
        app.snapshot = RuntimeSnapshot {
            estimated_history_tokens: 12_345,
            context_window_tokens: Some(22_537),
            reserved_output_tokens: 2_000,
            memory_selection: crate::context::MemorySelectionContextView {
                selection_budget_tokens: Some(8_192),
                selected_items: vec![
                    crate::context::MemorySelectionItemContextEntry {
                        order: 1,
                        kind: "workspace_memory".to_string(),
                        label: "Workspace Memory".to_string(),
                        detail: "memory".to_string(),
                        selection_reason: "active".to_string(),
                        budget_impact_tokens: Some(24),
                        dropped_reason: None,
                    },
                    crate::context::MemorySelectionItemContextEntry {
                        order: 2,
                        kind: "retrieved_workspace_memory".to_string(),
                        label: "Retrieved Experience".to_string(),
                        detail: "query=bootstrap".to_string(),
                        selection_reason: "retrieved".to_string(),
                        budget_impact_tokens: Some(18),
                        dropped_reason: None,
                    },
                ],
                available_items: Vec::new(),
                dropped_items: Vec::new(),
            },
            compaction_count: 2,
            last_compaction_before_tokens: Some(12_000),
            last_compaction_after_tokens: Some(4_500),
            last_compaction_recent_files: vec![
                "src/agent/compact.rs".to_string(),
                "crates/instructions/src/prompt.rs".to_string(),
            ],
            last_compaction_boundary_version: Some(1),
            last_compaction_boundary_before_tokens: Some(12_000),
            last_compaction_boundary_recent_file_count: Some(2),
            ..RuntimeSnapshot::default()
        };

        let rendered = status_resources_text(&app);
        assert!(rendered.contains("retrieval_budget=8192"));
        assert!(
            rendered.contains("retrieval_selected=workspace_memory, retrieved_workspace_memory")
        );
        assert!(rendered.contains("compactions=2 (last: 12000 -> 4500, ratio: 0.38)"));
        assert!(rendered.contains("compact_boundary=v1 (before_tokens=12000, recent_file_count=2)"));
        assert!(rendered.contains("recent_compact_file_count=2"));
        assert!(rendered.contains(
            "recent_compact_files=src/agent/compact.rs, crates/instructions/src/prompt.rs"
        ));
    }

    #[test]
    fn status_prompt_sources_text_uses_structured_prompt_source_entries() {
        let dir = tempdir().expect("tempdir");
        let cm = ConfigManager {
            path: dir.path().join("config.json"),
        };
        let mut app = TuiApp::new(cm).expect("app");
        app.snapshot.prompt_base_kind = "default".to_string();
        app.snapshot.prompt_section_keys =
            vec!["instructions".to_string(), "runtime_context".to_string()];
        app.snapshot.prompt_source_entries = vec![PromptSourceContextEntry {
            order: 1,
            kind: "project_instruction".to_string(),
            label: "Project Instruction (AGENTS.md)".to_string(),
            display_path: "AGENTS.md".to_string(),
            status_line: "project instruction: AGENTS.md".to_string(),
            inclusion_reason: "included because workspace instruction discovery found this file in the active workspace ancestry".to_string(),
        }];

        let rendered = status_prompt_sources_text(&app);
        assert!(
            rendered.contains("1. Project Instruction (AGENTS.md) [project_instruction] AGENTS.md")
        );
        assert!(rendered
            .contains("why: included because workspace instruction discovery found this file"));
    }

    #[test]
    fn status_runtime_text_reports_effective_provider_surface_sources() {
        let dir = tempdir().expect("tempdir");
        let cm = ConfigManager {
            path: dir.path().join("config.json"),
        };
        let mut app = TuiApp::new(cm).expect("app");
        app.config.set_provider("codex");

        let rendered = status_runtime_text(&app);
        assert!(rendered.contains("model_source="));
        assert!(rendered.contains("base_url_source="));
        assert!(rendered.contains("revision_source="));
        assert!(rendered.contains("reasoning_summary_source=legacy_global"));
    }

    #[test]
    fn status_context_text_includes_prompt_sources_and_plan_state() {
        let dir = tempdir().expect("tempdir");
        let cm = ConfigManager {
            path: dir.path().join("config.json"),
        };
        let mut app = TuiApp::new(cm).expect("app");
        app.snapshot = RuntimeSnapshot {
            cwd: "/workspace/rara".into(),
            branch: "main".into(),
            session_id: "session-123".into(),
            estimated_history_tokens: 12_345,
            context_window_tokens: Some(200_000),
            compact_threshold_tokens: 180_000,
            reserved_output_tokens: 8_192,
            stable_instructions_budget: 1_200,
            workspace_prompt_budget: 320,
            active_turn_budget: 280,
            compacted_history_budget: 140,
            retrieved_memory_budget: 96,
            remaining_input_budget: Some(189_772),
            compaction_count: 1,
            last_compaction_before_tokens: Some(12_000),
            last_compaction_after_tokens: Some(4_500),
            last_compaction_boundary_version: Some(1),
            last_compaction_boundary_before_tokens: Some(12_000),
            last_compaction_boundary_recent_file_count: Some(2),
            compaction_source_entries: vec![crate::context::CompactionSourceContextEntry {
                order: 1,
                kind: "compacted_summary".into(),
                label: "Compacted Thread Summary".into(),
                detail: "User Intent".into(),
                inclusion_reason:
                    "included because older thread history was compacted into a structured summary instead of being replayed verbatim".into(),
            }],
            retrieval_source_entries: vec![
                crate::context::RetrievalSourceContextEntry {
                    order: 1,
                    kind: "workspace_memory".into(),
                    label: "Workspace Memory".into(),
                    status: "active".into(),
                    detail: "/workspace/rara/.rara/memory.md".into(),
                    inclusion_reason:
                        "included now because the local workspace memory file was discovered as an explicit prompt source".into(),
                },
                crate::context::RetrievalSourceContextEntry {
                    order: 2,
                    kind: "thread_history".into(),
                    label: "Thread History".into(),
                    status: "available".into(),
                    detail: "session=session-123 messages=4".into(),
                    inclusion_reason:
                        "available as the session-local history source for restore and future recall surfaces".into(),
                },
            ],
            memory_selection: crate::context::MemorySelectionContextView {
                selection_budget_tokens: Some(1_024),
                selected_items: vec![
                    crate::context::MemorySelectionItemContextEntry {
                        order: 1,
                        kind: "workspace_memory".into(),
                        label: "Workspace Memory".into(),
                        detail: "/workspace/rara/.rara/memory.md; 3 line(s); first line: # Team Notes"
                            .into(),
                        selection_reason:
                            "selected because the current effective prompt includes the workspace memory file as an active input".into(),
                        budget_impact_tokens: Some(64),
                        dropped_reason: None,
                    },
                    crate::context::MemorySelectionItemContextEntry {
                        order: 2,
                        kind: "compacted_summary".into(),
                        label: "Compacted Thread Summary".into(),
                        detail: "User Intent".into(),
                        selection_reason:
                            "included because older thread history was compacted into a structured summary instead of being replayed verbatim".into(),
                        budget_impact_tokens: Some(48),
                        dropped_reason: None,
                    },
                    crate::context::MemorySelectionItemContextEntry {
                        order: 3,
                        kind: "retrieved_workspace_memory".into(),
                        label: "Retrieved Experience".into(),
                        detail: "query=bootstrap contract; recalled=2 item(s); preview: Prefer one shared bootstrap path. | Keep session restore aligned with direct execution.".into(),
                        selection_reason:
                            "selected because the retrieval tool returned relevant durable memory candidates for the current task".into(),
                        budget_impact_tokens: Some(32),
                        dropped_reason: None,
                    },
                ],
                available_items: vec![crate::context::MemorySelectionItemContextEntry {
                    order: 1,
                    kind: "thread_history".into(),
                    label: "Thread History".into(),
                    detail: "session=session-123 messages=4".into(),
                    selection_reason:
                        "thread history remains available as a recall source even when only active-turn state is currently injected".into(),
                    budget_impact_tokens: None,
                    dropped_reason: Some(
                        "raw thread history was not selected directly because the current turn already has sufficient active-turn and compacted-history context".into(),
                    ),
                }],
                dropped_items: vec![crate::context::MemorySelectionItemContextEntry {
                    order: 1,
                    kind: "retrieved_workspace_memory".into(),
                    label: "Retrieved Experience".into(),
                    detail: "query=bootstrap contract; recalled=5 item(s)".into(),
                    selection_reason:
                        "selected because the retrieval tool returned relevant durable memory candidates for the current task".into(),
                    budget_impact_tokens: Some(2_048),
                    dropped_reason: Some(
                        "not selected because it would exceed the remaining memory-selection budget (2048 > 1024)".into(),
                    ),
                }],
            },
            prompt_base_kind: "codex".into(),
            prompt_section_keys: vec!["stable_instructions".into(), "workspace".into()],
            prompt_source_entries: vec![
                crate::context::PromptSourceContextEntry {
                    order: 1,
                    kind: "project_instruction".into(),
                    label: "Project Instruction (AGENTS.md)".into(),
                    display_path: "AGENTS.md".into(),
                    status_line: "project instruction: AGENTS.md".into(),
                    inclusion_reason:
                        "included as a repository instruction discovered while walking from the workspace root toward the current focus directory".into(),
                },
                crate::context::PromptSourceContextEntry {
                    order: 2,
                    kind: "local_instruction".into(),
                    label: "RARA Local Instruction".into(),
                    display_path: "/workspace/rara/.rara/instructions.md".into(),
                    status_line: "local instruction: /workspace/rara/.rara/instructions.md".into(),
                    inclusion_reason:
                        "included as a workspace-local RARA instruction override".into(),
                },
            ],
            prompt_source_status_lines: vec![
                "AGENTS.md from workspace root".into(),
                "workspace instruction file".into(),
            ],
            assembly_entries: vec![
                crate::context::ContextAssemblyEntry {
                    order: 1,
                    layer: "stable_instructions".into(),
                    kind: "project_instruction".into(),
                    label: "Project Instruction (AGENTS.md)".into(),
                    source_path: Some("AGENTS.md".into()),
                    injected: true,
                    inclusion_reason:
                        "included as a repository instruction discovered while walking from the workspace root toward the current focus directory".into(),
                    budget_impact_tokens: Some(120),
                    dropped_reason: None,
                },
                crate::context::ContextAssemblyEntry {
                    order: 2,
                    layer: "workspace_prompt_sources".into(),
                    kind: "local_memory".into(),
                    label: "Workspace Memory".into(),
                    source_path: Some("/workspace/rara/.rara/memory.md".into()),
                    injected: true,
                    inclusion_reason:
                        "selected because the current effective prompt includes the workspace memory file as an active input".into(),
                    budget_impact_tokens: Some(64),
                    dropped_reason: None,
                },
                crate::context::ContextAssemblyEntry {
                    order: 3,
                    layer: "active_memory_inputs".into(),
                    kind: "retrieved_workspace_memory".into(),
                    label: "Retrieved Experience".into(),
                    source_path: None,
                    injected: true,
                    inclusion_reason:
                        "selected because the retrieval tool returned relevant durable memory candidates for the current task".into(),
                    budget_impact_tokens: Some(32),
                    dropped_reason: None,
                },
                crate::context::ContextAssemblyEntry {
                    order: 4,
                    layer: "compacted_history".into(),
                    kind: "compacted_summary".into(),
                    label: "Compacted Thread Summary".into(),
                    source_path: None,
                    injected: true,
                    inclusion_reason:
                        "included because older thread history was compacted into a structured summary instead of being replayed verbatim".into(),
                    budget_impact_tokens: Some(48),
                    dropped_reason: None,
                },
                crate::context::ContextAssemblyEntry {
                    order: 5,
                    layer: "active_turn_state".into(),
                    kind: "plan_steps".into(),
                    label: "Plan Steps".into(),
                    source_path: None,
                    injected: true,
                    inclusion_reason:
                        "included because structured plan steps are part of the current active thread state".into(),
                    budget_impact_tokens: Some(80),
                    dropped_reason: None,
                },
                crate::context::ContextAssemblyEntry {
                    order: 6,
                    layer: "retrieval_ready".into(),
                    kind: "thread_history".into(),
                    label: "Thread History".into(),
                    source_path: None,
                    injected: false,
                    inclusion_reason:
                        "available as the session-local history source for restore and future recall surfaces".into(),
                    budget_impact_tokens: None,
                    dropped_reason:
                        Some("available for recall, but not selected into the current assembled context".into()),
                },
            ],
            plan_steps: vec![("pending".into(), "Implement /context".into())],
            ..RuntimeSnapshot::default()
        };

        let rendered = status_context_text(&app);
        assert!(rendered.contains("Current Session"));
        assert!(rendered.contains("Context Budget"));
        assert!(rendered.contains("stable instructions: 1200"));
        assert!(rendered.contains("Stable Instructions"));
        assert!(rendered.contains("Workspace Prompt Sources"));
        assert!(rendered.contains("Active Memory Inputs"));
        assert!(rendered.contains("Memory Selection"));
        assert!(rendered.contains("selection budget: 1024"));
        assert!(rendered.contains("Compacted History"));
        assert!(rendered.contains("Active Turn State"));
        assert!(rendered.contains("Retrieval-ready but not injected items"));
        assert!(rendered.contains("available but not injected"));
        assert!(rendered.contains("dropped by ranking or budget"));
        assert!(rendered.contains("1. Project Instruction (AGENTS.md) (project_instruction)"));
        assert!(rendered.contains("path: AGENTS.md"));
        assert!(rendered.contains("injected: yes"));
        assert!(rendered.contains("budget impact: 120"));
        assert!(rendered.contains("dropped: -"));
        assert!(rendered.contains("6. Thread History (thread_history)"));
        assert!(rendered.contains("injected: no"));
        assert!(rendered.contains("not injected: raw thread history was not selected directly"));
        assert!(rendered.contains(
            "dropped: not selected because it would exceed the remaining memory-selection budget"
        ));
        assert!(rendered.contains("why selected: selected because the current effective prompt includes the workspace memory file as an active input"));
        assert!(rendered.contains("[pending] Implement /context"));
    }

    #[test]
    fn status_runtime_text_reports_model_and_reasoning_sources() {
        let dir = tempdir().expect("tempdir");
        let mut app = TuiApp::new(ConfigManager {
            path: dir.path().join("config.json"),
        })
        .expect("app");

        app.config.set_provider("openai-compatible");
        app.config
            .set_base_url(Some("http://proxy.local/v1".to_string()));
        app.config.set_model(Some("custom-model".to_string()));
        app.config
            .set_reasoning_summary(Some("detailed".to_string()));

        let rendered = status_runtime_text(&app);
        assert!(rendered.contains("endpoint_profile=Custom endpoint"));
        assert!(rendered.contains("endpoint_kind=custom"));
        assert!(rendered.contains("model=custom-model"));
        assert!(rendered.contains("model_source=provider_state"));
        assert!(rendered.contains("base_url_source=provider_state"));
        assert!(rendered.contains("api_key_source=unset"));
        assert!(rendered.contains("reasoning_summary=detailed"));
        assert!(rendered.contains("reasoning_summary_source=provider_state"));
        assert!(rendered.contains("reasoning_effort_source=unset"));
        assert!(rendered.contains("revision_source=unset"));
    }

    #[test]
    fn status_runtime_text_reports_active_openai_endpoint_profile() {
        let dir = tempdir().expect("tempdir");
        let mut app = TuiApp::new(ConfigManager {
            path: dir.path().join("config.json"),
        })
        .expect("app");

        app.config.select_openai_profile(
            "openrouter-main",
            "OpenRouter main",
            rara_config::OpenAiEndpointKind::Openrouter,
        );
        app.config
            .set_model(Some("anthropic/claude-sonnet-4".to_string()));

        let rendered = status_runtime_text(&app);
        assert!(rendered.contains("provider=openai-compatible"));
        assert!(rendered.contains("endpoint_profile=OpenRouter main"));
        assert!(rendered.contains("endpoint_kind=openrouter"));
        assert!(rendered.contains("model=anthropic/claude-sonnet-4"));
    }

    #[test]
    fn status_runtime_text_reports_codex_auth_surface() {
        let dir = tempdir().expect("tempdir");
        let mut app = TuiApp::new(ConfigManager {
            path: dir.path().join("config.json"),
        })
        .expect("app");

        app.config.set_provider("codex");
        app.config.set_base_url(Some(
            rara_config::DEFAULT_CODEX_CHATGPT_BASE_URL.to_string(),
        ));
        app.config
            .set_model(Some(rara_config::DEFAULT_CODEX_MODEL.to_string()));
        app.codex_auth_mode = Some(crate::oauth::SavedCodexAuthMode::Chatgpt);

        let rendered = status_runtime_text(&app);
        assert!(rendered.contains("codex_auth_mode=chatgpt"));
        assert!(rendered.contains("codex_endpoint_kind=chatgpt_codex"));
    }
}
