use super::super::state::{
    TuiApp, PendingInteractionSnapshot, RuntimeSnapshot,
    TranscriptEntry, TranscriptEntryPayload,
};

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
        render_context_assembly_entries(
            app,
            "workspace_prompt_sources",
            "Workspace Prompt Sources",
        ),
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
        surface
            .reasoning_effort
            .display_or(reasoning_effort_label.as_str()),
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
