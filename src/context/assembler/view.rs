use super::RuntimeInteractionInput;
use super::compaction;
use super::memory_selection::memory_selection;
use crate::agent::{Message, PlanStepStatus};
use crate::context::{
    CompactionSourceContextEntry, ContextAssemblyEntry, ContextAssemblyView,
    MemorySelectionContextView, MemorySelectionItemContextEntry, RetrievalContextView,
    RetrievalSourceContextEntry,
};
use crate::prompt::EffectivePrompt;
use crate::workspace::WorkspaceMemory;

pub(crate) fn estimate_text_tokens(text: &str) -> usize {
    // Token estimation is inherently approximate; use a coarse heuristic:
    // ~75% of a whitespace-split word count, clamped to the character count.
    let word_count = text.split_whitespace().count();
    text.len().min((word_count * 4) / 3)
}

pub(crate) fn latest_user_request(history: &[Message]) -> Option<String> {
    history
        .iter()
        .rev()
        .filter(|message| message.role == "user")
        .find_map(extract_latest_text)
}

pub(crate) fn latest_tool_results(history: &[Message]) -> Vec<(String, String)> {
    history
        .iter()
        .rev()
        .find(|message| message.role == "user" && message.content.as_array().is_some())
        .and_then(|message| message.content.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    if item.get("type").and_then(serde_json::Value::as_str) != Some("tool_result") {
                        return None;
                    }
                    let tool_use_id = item
                        .get("tool_use_id")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("unknown");
                    let detail = item
                        .get("content")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("-");
                    Some((
                        format!("Tool Result: {tool_use_id}"),
                        truncate_tool_detail(detail),
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn extract_latest_text(message: &Message) -> Option<String> {
    message.content.as_array().and_then(|items| {
        items
            .iter()
            .filter(|item| {
                item.get("type")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|value| value == "text")
            })
            .filter_map(|item| item.get("text").and_then(serde_json::Value::as_str))
            .map(str::to_string)
            .reduce(|accumulated, next| accumulated + "\n" + &next)
    })
}

fn truncate_tool_detail(detail: &str) -> String {
    if detail.len() > 256 {
        format!("{}...", &detail[..252])
    } else {
        detail.to_string()
    }
}

pub(crate) fn assemble_context_view(
    effective_prompt: &EffectivePrompt,
    plan_explanation: Option<&str>,
    plan_steps: &[(PlanStepStatus, String)],
    pending_interactions: &[RuntimeInteractionInput],
    compaction_entries: &[CompactionSourceContextEntry],
    selected_memory_items: &[MemorySelectionItemContextEntry],
    available_memory_items: &[MemorySelectionItemContextEntry],
    dropped_memory_items: &[MemorySelectionItemContextEntry],
    history: &[Message],
) -> ContextAssemblyView {
    let mut entries = Vec::new();

    let mut push = |mut entry: ContextAssemblyEntry| {
        entry.order = entries.len() + 1;
        entries.push(entry);
    };

    push(ContextAssemblyEntry {
        order: 0,
        layer: "stable_instructions".to_string(),
        kind: format!("base_prompt:{}", effective_prompt.base_prompt_kind.label()),
        label: "Base System Prompt".to_string(),
        source_path: None,
        injected: true,
        inclusion_reason: "included as the stable system prompt scaffold for every turn"
            .to_string(),
        budget_impact_tokens: Some(estimate_text_tokens(effective_prompt.text.as_str())),
        dropped_reason: None,
    });

    for source in &effective_prompt.sources {
        let kind = source.kind_label().to_string();
        let layer = match kind.as_str() {
            "project_instruction" => "stable_instructions",
            "local_memory" => "workspace_prompt_sources",
            _ => "workspace_prompt_sources",
        };
        push(ContextAssemblyEntry {
            order: 0,
            layer: layer.to_string(),
            kind,
            label: source.label.clone(),
            source_path: Some(source.display_path.clone()),
            injected: true,
            inclusion_reason: source.inclusion_reason().to_string(),
            budget_impact_tokens: Some(estimate_text_tokens(source.content.as_str())),
            dropped_reason: None,
        });
    }

    if let Some(plan_explanation) = plan_explanation.filter(|value| !value.trim().is_empty()) {
        push(ContextAssemblyEntry {
            order: 0,
            layer: "active_turn_state".to_string(),
            kind: "plan_explanation".to_string(),
            label: "Plan Explanation".to_string(),
            source_path: None,
            injected: true,
            inclusion_reason:
                "included because the active thread currently carries a structured plan explanation"
                    .to_string(),
            budget_impact_tokens: Some(estimate_text_tokens(plan_explanation)),
            dropped_reason: None,
        });
    }

    if !plan_steps.is_empty() {
        let detail = plan_steps
            .iter()
            .map(|(_, step)| step.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        push(ContextAssemblyEntry {
            order: 0,
            layer: "active_turn_state".to_string(),
            kind: "plan_steps".to_string(),
            label: "Plan Steps".to_string(),
            source_path: None,
            injected: true,
            inclusion_reason:
                "included because structured plan steps are part of the current active thread"
                    .to_string(),
            budget_impact_tokens: Some(estimate_text_tokens(detail.as_str())),
            dropped_reason: None,
        });
    }

    for interaction in pending_interactions {
        push(ContextAssemblyEntry {
            order: 0,
            layer: "active_turn_state".to_string(),
            kind: interaction.kind.clone(),
            label: interaction.title.clone(),
            source_path: interaction.source.clone(),
            injected: true,
            inclusion_reason:
                "included because pending interactions are active runtime obligations".to_string(),
            budget_impact_tokens: Some(
                estimate_text_tokens(interaction.title.as_str())
                    + estimate_text_tokens(interaction.summary.as_str()),
            ),
            dropped_reason: None,
        });
    }

    if let Some(user_request) = latest_user_request(history) {
        push(ContextAssemblyEntry {
            order: 0,
            layer: "active_turn_state".to_string(),
            kind: "latest_user_request".to_string(),
            label: "Latest User Request".to_string(),
            source_path: None,
            injected: true,
            inclusion_reason:
                "included because the latest user request anchors the current turn objective"
                    .to_string(),
            budget_impact_tokens: Some(estimate_text_tokens(user_request.as_str())),
            dropped_reason: None,
        });
    }

    for (label, detail) in latest_tool_results(history) {
        push(ContextAssemblyEntry {
            order: 0,
            layer: "active_turn_state".to_string(),
            kind: "tool_result".to_string(),
            label,
            source_path: None,
            injected: true,
            inclusion_reason: "included because recent tool results are part of the current working set for the active turn".to_string(),
            budget_impact_tokens: Some(estimate_text_tokens(detail.as_str())),
            dropped_reason: None,
        });
    }

    for entry in compaction_entries {
        push(ContextAssemblyEntry {
            order: 0,
            layer: "compacted_history".to_string(),
            kind: entry.kind.clone(),
            label: entry.label.clone(),
            source_path: None,
            injected: true,
            inclusion_reason: entry.inclusion_reason.clone(),
            budget_impact_tokens: Some(estimate_text_tokens(entry.detail.as_str())),
            dropped_reason: None,
        });
    }

    for item in selected_memory_items {
        push(ContextAssemblyEntry {
            order: 0,
            layer: "active_memory_inputs".to_string(),
            kind: item.kind.clone(),
            label: item.label.clone(),
            source_path: None,
            injected: true,
            inclusion_reason: item.selection_reason.clone(),
            budget_impact_tokens: item.budget_impact_tokens,
            dropped_reason: None,
        });
    }

    for item in available_memory_items {
        push(ContextAssemblyEntry {
            order: 0,
            layer: "retrieval_ready".to_string(),
            kind: item.kind.clone(),
            label: item.label.clone(),
            source_path: None,
            injected: false,
            inclusion_reason: item.selection_reason.clone(),
            budget_impact_tokens: item.budget_impact_tokens,
            dropped_reason: item.dropped_reason.clone(),
        });
    }

    for item in dropped_memory_items {
        push(ContextAssemblyEntry {
            order: 0,
            layer: "retrieval_ready".to_string(),
            kind: item.kind.clone(),
            label: item.label.clone(),
            source_path: None,
            injected: false,
            inclusion_reason: item.selection_reason.clone(),
            budget_impact_tokens: item.budget_impact_tokens,
            dropped_reason: item.dropped_reason.clone(),
        });
    }

    ContextAssemblyView { entries }
}

pub(crate) fn active_turn_budget(
    plan_explanation: Option<&str>,
    plan_steps: &[(PlanStepStatus, String)],
    pending_interactions: &[RuntimeInteractionInput],
    history: &[Message],
) -> usize {
    let plan_budget = plan_explanation
        .map(estimate_text_tokens)
        .unwrap_or_default()
        + plan_steps
            .iter()
            .map(|(_, step)| estimate_text_tokens(step.as_str()))
            .sum::<usize>();
    let interaction_budget = pending_interactions
        .iter()
        .map(|interaction| {
            estimate_text_tokens(interaction.title.as_str())
                + estimate_text_tokens(interaction.summary.as_str())
        })
        .sum::<usize>();
    let latest_request_budget = latest_user_request(history)
        .map(|value| estimate_text_tokens(value.as_str()))
        .unwrap_or_default();
    let tool_budget = latest_tool_results(history)
        .into_iter()
        .map(|(_, detail)| estimate_text_tokens(detail.as_str()))
        .sum::<usize>();

    plan_budget + interaction_budget + latest_request_budget + tool_budget
}

pub(crate) fn compaction_context_entries(history: &[Message]) -> Vec<CompactionSourceContextEntry> {
    let source_items = compaction::compaction_source_entries(history);
    source_items
        .into_iter()
        .map(|item| CompactionSourceContextEntry {
            order: item.order,
            kind: item.kind,
            label: item.label,
            detail: item.detail,
            inclusion_reason: item.inclusion_reason,
        })
        .collect()
}

pub(crate) fn retrieval_context(
    prompt_sources: &[crate::prompt::PromptSource],
    plan_explanation: Option<&str>,
    plan_steps: &[(PlanStepStatus, String)],
    pending_interactions: &[RuntimeInteractionInput],
    history: &[Message],
    session_id: &str,
    vdb_uri: &str,
    compact_state: &crate::agent::CompactState,
    workspace: &WorkspaceMemory,
) -> RetrievalContextView {
    let selection_budget_tokens =
        memory_selection_budget_tokens(compact_state.context_window_tokens);
    let memory_selection_view = memory_selection(
        prompt_sources,
        plan_explanation,
        plan_steps,
        pending_interactions,
        history,
        session_id,
        vdb_uri,
        selection_budget_tokens,
    );

    let workspace_memory_path = workspace.rara_dir.join("memory.md");
    let workspace_memory_active = workspace_memory_path.exists();
    let workspace_memory_available = workspace_memory_active && workspace.has_memory_file_cached();
    let workspace_memory_display = workspace_memory_path.display().to_string();
    let workspace_memory_status = if workspace_memory_active {
        "active"
    } else if workspace_memory_available {
        "available"
    } else {
        "missing"
    };

    let thread_history_status = if memory_selection_view
        .selected_items
        .iter()
        .any(|item| item.kind == "thread_history")
    {
        "active"
    } else if !history.is_empty() {
        "available"
    } else {
        "missing"
    };

    let vector_memory_status = if vdb_uri.is_empty() {
        "missing"
    } else if memory_selection_view
        .selected_items
        .iter()
        .any(|item| item.kind == "vector_memory")
    {
        "active"
    } else {
        "available"
    };

    RetrievalContextView {
        memory_selection: memory_selection_view,
        entries: vec![
            RetrievalSourceContextEntry {
                order: 1,
                kind: "workspace_memory".to_string(),
                label: "Workspace Memory".to_string(),
                status: workspace_memory_status.to_string(),
                detail: workspace_memory_display,
                inclusion_reason: match workspace_memory_status {
                    "active" => "included now because the local workspace memory file was discovered as an explicit prompt source".to_string(),
                    "available" => "available for future recall or prompt injection, but not active in the current turn".to_string(),
                    _ => "no workspace memory file is available for recall or prompt injection".to_string(),
                },
            },
            RetrievalSourceContextEntry {
                order: 2,
                kind: "thread_history".to_string(),
                label: "Thread History".to_string(),
                status: thread_history_status.to_string(),
                detail: format!("session={session_id} messages={}", history.len()),
                inclusion_reason: if history.is_empty() {
                    "no persisted thread history is available for session-local recall yet".to_string()
                } else {
                    "available as the session-local history source for restore and future recall surfaces".to_string()
                },
            },
            RetrievalSourceContextEntry {
                order: 3,
                kind: "vector_memory".to_string(),
                label: "Vector Memory Store".to_string(),
                status: vector_memory_status.to_string(),
                detail: vdb_uri.to_string(),
                inclusion_reason: if vector_memory_status == "available" {
                    "configured as the durable vector-backed memory store for later retrieval, even though the current recall path is still limited".to_string()
                } else {
                    "no vector-backed memory store is configured for retrieval".to_string()
                },
            },
        ],
    }
}

fn memory_selection_budget_tokens(context_window_tokens: Option<usize>) -> Option<usize> {
    let window = context_window_tokens?;
    let budget = window.saturating_mul(5) / 100;
    Some(budget)
}
