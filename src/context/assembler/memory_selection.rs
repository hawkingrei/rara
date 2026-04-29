use super::compaction::{self, CompactionSourceItem};
use super::view::{estimate_text_tokens, latest_tool_results, latest_user_request};
use crate::agent::{Message, PlanStepStatus};
use crate::context::{MemorySelectionContextView, MemorySelectionItemContextEntry};
use crate::prompt::PromptSource;
use serde_json::Value;
use std::collections::HashMap;

pub(crate) fn memory_selection(
    prompt_sources: &[PromptSource],
    plan_explanation: Option<&str>,
    plan_steps: &[(PlanStepStatus, String)],
    pending_interactions: &[super::RuntimeInteractionInput],
    history: &[Message],
    session_id: &str,
    vdb_uri: &str,
    selection_budget_tokens: Option<usize>,
) -> MemorySelectionContextView {
    let mut selected_items = fixed_memory_selection_items(
        prompt_sources,
        plan_explanation,
        plan_steps,
        pending_interactions,
        history,
    );
    let fixed_kinds = selected_items
        .iter()
        .map(|item| item.kind.clone())
        .collect::<Vec<_>>();
    let mut discretionary = select_memory_candidates(
        retrieval_memory_candidates(history, session_id, vdb_uri),
        selection_budget_tokens,
        fixed_kinds.as_slice(),
    );
    selected_items.append(&mut discretionary.selected_items);
    for (idx, item) in selected_items.iter_mut().enumerate() {
        item.order = idx + 1;
    }

    let mut available_items = discretionary.available_items;
    if !selected_items
        .iter()
        .any(|item| item.kind == "workspace_memory")
    {
        available_items.push(workspace_memory_available_item(prompt_sources));
    }
    for (idx, item) in available_items.iter_mut().enumerate() {
        item.order = idx + 1;
    }

    let mut dropped_items = discretionary.dropped_items;
    for (idx, item) in dropped_items.iter_mut().enumerate() {
        item.order = idx + 1;
    }

    MemorySelectionContextView {
        selection_budget_tokens,
        selected_items,
        available_items,
        dropped_items,
    }
}

// -- candidate / decision types --

#[derive(Debug, Clone)]
struct MemorySelectionCandidate {
    kind: String,
    label: String,
    detail: String,
    selection_reason: String,
    budget_impact_tokens: Option<usize>,
    priority: usize,
    selectable: bool,
    dropped_reason: String,
}

#[derive(Debug, Default)]
struct MemorySelectionDecision {
    selected_items: Vec<MemorySelectionItemContextEntry>,
    available_items: Vec<MemorySelectionItemContextEntry>,
    dropped_items: Vec<MemorySelectionItemContextEntry>,
}

// -- fixed selection items (always selected) --

fn fixed_memory_selection_items(
    prompt_sources: &[PromptSource],
    plan_explanation: Option<&str>,
    plan_steps: &[(PlanStepStatus, String)],
    pending_interactions: &[super::RuntimeInteractionInput],
    history: &[Message],
) -> Vec<MemorySelectionItemContextEntry> {
    let mut items = workspace_memory_selected_items(prompt_sources);
    items.append(&mut compacted_history_selected_items(history));

    if let Some(plan_explanation) = plan_explanation.filter(|value| !value.trim().is_empty()) {
        items.push(MemorySelectionItemContextEntry {
            order: 0,
            kind: "plan_explanation".to_string(),
            label: "Plan Explanation".to_string(),
            detail: plan_explanation.to_string(),
            selection_reason:
                "selected because the active thread currently carries a structured plan explanation"
                    .to_string(),
            budget_impact_tokens: Some(estimate_text_tokens(plan_explanation)),
            dropped_reason: None,
        });
    }

    if !plan_steps.is_empty() {
        let detail = plan_steps
            .iter()
            .map(|(status, step)| {
                let status = match status {
                    PlanStepStatus::Pending => "pending",
                    PlanStepStatus::InProgress => "in_progress",
                    PlanStepStatus::Completed => "completed",
                };
                format!("[{status}] {step}")
            })
            .collect::<Vec<_>>()
            .join("\n");
        items.push(MemorySelectionItemContextEntry {
            order: 0,
            kind: "plan_steps".to_string(),
            label: "Plan Steps".to_string(),
            detail: detail.clone(),
            selection_reason: "selected because structured plan steps are part of the current thread working set and must survive restore".to_string(),
            budget_impact_tokens: Some(estimate_text_tokens(detail.as_str())),
            dropped_reason: None,
        });
    }

    for interaction in pending_interactions {
        items.push(MemorySelectionItemContextEntry {
            order: 0,
            kind: interaction.kind.clone(),
            label: interaction.title.clone(),
            detail: interaction.summary.clone(),
            selection_reason: "selected because pending interactions are active runtime obligations that must remain available until answered".to_string(),
            budget_impact_tokens: Some(
                estimate_text_tokens(interaction.title.as_str())
                    + estimate_text_tokens(interaction.summary.as_str()),
            ),
            dropped_reason: None,
        });
    }

    if let Some(user_request) = latest_user_request(history) {
        items.push(MemorySelectionItemContextEntry {
            order: 0,
            kind: "latest_user_request".to_string(),
            label: "Latest User Request".to_string(),
            detail: user_request.clone(),
            selection_reason: "selected because the latest user request anchors the current turn objective and should stay in the active working set".to_string(),
            budget_impact_tokens: Some(estimate_text_tokens(user_request.as_str())),
            dropped_reason: None,
        });
    }

    for (label, detail) in latest_tool_results(history) {
        items.push(MemorySelectionItemContextEntry {
            order: 0,
            kind: "tool_result".to_string(),
            label,
            detail: detail.clone(),
            selection_reason: "selected because recent tool results are part of the active thread working set until the assistant synthesizes a final answer".to_string(),
            budget_impact_tokens: Some(estimate_text_tokens(detail.as_str())),
            dropped_reason: None,
        });
    }

    items
}

fn workspace_memory_selected_items(
    prompt_sources: &[PromptSource],
) -> Vec<MemorySelectionItemContextEntry> {
    prompt_sources
        .iter()
        .filter(|source| source.kind_label() == "local_memory")
        .map(workspace_memory_selected_item)
        .collect()
}

fn workspace_memory_selected_item(source: &PromptSource) -> MemorySelectionItemContextEntry {
    MemorySelectionItemContextEntry {
        order: 0,
        kind: "workspace_memory".to_string(),
        label: "Workspace Memory".to_string(),
        detail: format!(
            "{}; {}",
            source.display_path,
            compaction::summarize_workspace_memory_source(source.content.as_str())
        ),
        selection_reason: "selected because the current effective prompt includes the workspace memory file as an active input".to_string(),
        budget_impact_tokens: Some(estimate_text_tokens(source.content.as_str())),
        dropped_reason: None,
    }
}

fn compacted_history_selected_items(history: &[Message]) -> Vec<MemorySelectionItemContextEntry> {
    compaction::compaction_source_entries(history)
        .into_iter()
        .filter(|entry| entry.kind != "compact_boundary")
        .map(
            |CompactionSourceItem {
                 kind,
                 label,
                 detail,
                 inclusion_reason,
                 ..
             }| MemorySelectionItemContextEntry {
                order: 0,
                kind,
                label,
                budget_impact_tokens: Some(estimate_text_tokens(detail.as_str())),
                detail,
                selection_reason: inclusion_reason,
                dropped_reason: None,
            },
        )
        .collect()
}

// -- discretionary retrieval candidates --

fn retrieval_memory_candidates(
    history: &[Message],
    session_id: &str,
    vdb_uri: &str,
) -> Vec<MemorySelectionCandidate> {
    let mut candidates = retrieval_tool_candidates(history);
    candidates.push(thread_history_candidate(history, session_id));
    candidates.push(vector_memory_candidate(vdb_uri));
    candidates
}

fn retrieval_tool_candidates(history: &[Message]) -> Vec<MemorySelectionCandidate> {
    let mut pending: HashMap<String, (String, Option<String>)> = HashMap::new();
    let mut items = Vec::new();
    for message in history.iter().rev() {
        if message.role == "user" {
            collect_retrieval_tool_results(&mut pending, &mut items, message);
        } else if message.role == "assistant" {
            collect_retrieval_tool_uses(&mut pending, message);
        }
    }
    items.reverse();
    items
}

fn collect_retrieval_tool_uses(
    pending: &mut HashMap<String, (String, Option<String>)>,
    message: &Message,
) {
    let Some(blocks) = message.content.as_array() else {
        return;
    };
    for item in blocks {
        let Some(name) = item.get("name").and_then(Value::as_str) else {
            continue;
        };
        if !matches!(name, "retrieve_experience" | "retrieve_session_context") {
            continue;
        }
        let Some(tool_use_id) = item.get("id").and_then(Value::as_str) else {
            continue;
        };
        let query = item
            .get("input")
            .and_then(Value::as_object)
            .and_then(|input| input.get("query"))
            .and_then(Value::as_str)
            .map(str::to_string);
        pending.insert(tool_use_id.to_string(), (name.to_string(), query));
    }
}

fn collect_retrieval_tool_results(
    pending: &mut HashMap<String, (String, Option<String>)>,
    items: &mut Vec<MemorySelectionCandidate>,
    message: &Message,
) {
    let Some(blocks) = message.content.as_array() else {
        return;
    };
    for block in blocks {
        let Some(item_type) = block.get("type").and_then(Value::as_str) else {
            continue;
        };
        if item_type != "tool_result" {
            continue;
        }
        let Some(tool_use_id) = block.get("tool_use_id").and_then(Value::as_str) else {
            continue;
        };
        let Some((name, query)) = pending.remove(tool_use_id) else {
            continue;
        };
        let content = block
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default();
        items.push(retrieval_tool_candidate(
            name.as_str(),
            query.as_deref(),
            content,
        ));
    }
}

fn select_memory_candidates(
    mut candidates: Vec<MemorySelectionCandidate>,
    selection_budget_tokens: Option<usize>,
    fixed_selected_kinds: &[String],
) -> MemorySelectionDecision {
    const BUDGET_DROP_REASON_PREFIX: &str =
        "not selected because it would exceed the remaining memory-selection budget";
    candidates.sort_by_key(|candidate| candidate.priority);
    let mut remaining_budget = selection_budget_tokens;
    let has_compacted_history = fixed_selected_kinds.iter().any(|kind| {
        matches!(
            kind.as_str(),
            "compacted_summary" | "recent_files" | "recent_file_excerpts"
        )
    });
    let mut decision = MemorySelectionDecision::default();
    let mut selected_kinds = fixed_selected_kinds.to_vec();

    for candidate in candidates {
        let should_drop = if !candidate.selectable {
            Some(candidate.dropped_reason.clone())
        } else if candidate.kind == "thread_history" && has_compacted_history {
            Some(
                "not selected because compacted thread history already provides a more focused carried-over thread view".to_string(),
            )
        } else if selected_kinds
            .iter()
            .any(|kind| kind == "retrieved_thread_context" && candidate.kind == "thread_history")
        {
            Some(
                "not selected because a more focused retrieved thread-context candidate already won the current memory selection".to_string(),
            )
        } else if let (Some(remaining), Some(cost)) =
            (remaining_budget, candidate.budget_impact_tokens)
        {
            (cost > remaining)
                .then(|| format!("{BUDGET_DROP_REASON_PREFIX} ({cost} > {remaining})"))
        } else {
            None
        };

        if let Some(dropped_reason) = should_drop {
            let item = MemorySelectionItemContextEntry {
                order: 0,
                kind: candidate.kind,
                label: candidate.label,
                detail: candidate.detail,
                selection_reason: candidate.selection_reason,
                budget_impact_tokens: candidate.budget_impact_tokens,
                dropped_reason: Some(dropped_reason),
            };
            if item
                .dropped_reason
                .as_deref()
                .is_some_and(|reason| reason.starts_with(BUDGET_DROP_REASON_PREFIX))
            {
                decision.dropped_items.push(item);
            } else {
                decision.available_items.push(item);
            }
        } else {
            if let Some(cost) = candidate.budget_impact_tokens {
                remaining_budget = remaining_budget.map(|remaining| remaining.saturating_sub(cost));
            }
            decision
                .selected_items
                .push(MemorySelectionItemContextEntry {
                    order: 0,
                    kind: candidate.kind.clone(),
                    label: candidate.label,
                    detail: candidate.detail,
                    selection_reason: candidate.selection_reason,
                    budget_impact_tokens: candidate.budget_impact_tokens,
                    dropped_reason: None,
                });
            selected_kinds.push(candidate.kind);
        }
    }

    decision
}

// -- candidate factories --

fn workspace_memory_available_item(
    prompt_sources: &[PromptSource],
) -> MemorySelectionItemContextEntry {
    let workspace_memory_available = prompt_sources
        .iter()
        .any(|source| source.kind_label() == "local_memory");
    MemorySelectionItemContextEntry {
        order: 0,
        kind: "workspace_memory".to_string(),
        label: "Workspace Memory".to_string(),
        detail: "-".to_string(),
        selection_reason: if workspace_memory_available {
            "available for future recall or prompt injection".to_string()
        } else {
            "no workspace memory file is currently discoverable".to_string()
        },
        budget_impact_tokens: None,
        dropped_reason: Some(if workspace_memory_available {
            "available for recall, but not selected into the current turn because workspace memory was not activated as a prompt input".to_string()
        } else {
            "no workspace memory candidate is currently available".to_string()
        }),
    }
}

fn thread_history_candidate(history: &[Message], session_id: &str) -> MemorySelectionCandidate {
    MemorySelectionCandidate {
        kind: "thread_history".to_string(),
        label: "Thread History".to_string(),
        detail: format!("session={session_id} messages={}", history.len()),
        selection_reason: "thread history remains available as a recall source even when only active-turn state is currently injected".to_string(),
        budget_impact_tokens: Some(estimate_text_tokens(
            format!("session={session_id} messages={}", history.len()).as_str(),
        )),
        priority: 30,
        selectable: !history.is_empty(),
        dropped_reason: if history.is_empty() {
            "no thread history is available for selection".to_string()
        } else {
            "raw thread history was not selected directly because the current turn already has sufficient active-turn and compacted-history context".to_string()
        },
    }
}

fn vector_memory_candidate(vdb_uri: &str) -> MemorySelectionCandidate {
    MemorySelectionCandidate {
        kind: "vector_memory".to_string(),
        label: "Vector Memory Store".to_string(),
        detail: if vdb_uri.is_empty() {
            "-".to_string()
        } else {
            vdb_uri.to_string()
        },
        selection_reason: "the vector-backed memory slot is part of the selection contract even before full ranked retrieval is implemented".to_string(),
        budget_impact_tokens: None,
        priority: 40,
        selectable: false,
        dropped_reason: if vdb_uri.is_empty() {
            "no vector-backed memory store is configured".to_string()
        } else {
            "not selected because vector-backed candidate ranking is not implemented yet".to_string()
        },
    }
}

fn retrieval_tool_candidate(
    tool_name: &str,
    query: Option<&str>,
    content: &str,
) -> MemorySelectionCandidate {
    match tool_name {
        "retrieve_experience" => {
            let experiences =
                compaction::extract_json_array_strings(content, "relevant_experiences");
            let preview = if experiences.is_empty() {
                "no recalled experiences".to_string()
            } else {
                format!(
                    "recalled={} item(s); preview: {}",
                    experiences.len(),
                    experiences.join(" | ")
                )
            };
            let query = query.unwrap_or("query unavailable");
            let detail = format!("query={query}; {preview}");
            MemorySelectionCandidate {
                kind: "retrieved_workspace_memory".to_string(),
                label: "Retrieved Experience".to_string(),
                budget_impact_tokens: Some(estimate_text_tokens(detail.as_str())),
                detail,
                selection_reason: "selected because the retrieval tool returned relevant durable memory candidates for the current task".to_string(),
                priority: 10,
                selectable: true,
                dropped_reason: "not selected after ranking the retrieved workspace-memory candidates against the current memory-selection budget".to_string(),
            }
        }
        "retrieve_session_context" => {
            let summary = compaction::extract_json_string_field(content, "summary")
                .unwrap_or_else(|| "no session-context summary".to_string());
            let query = query.unwrap_or("query unavailable");
            let detail = format!("query={query}; summary: {summary}");
            MemorySelectionCandidate {
                kind: "retrieved_thread_context".to_string(),
                label: "Retrieved Thread Context".to_string(),
                budget_impact_tokens: Some(estimate_text_tokens(detail.as_str())),
                detail,
                selection_reason: "selected because the retrieval tool returned relevant thread-level context to help ground the current task".to_string(),
                priority: 20,
                selectable: true,
                dropped_reason: "not selected after ranking the retrieved thread-context candidates against the current memory-selection budget".to_string(),
            }
        }
        _ => MemorySelectionCandidate {
            kind: format!("retrieved_{tool_name}"),
            label: "Retrieved Content".to_string(),
            detail: "-".to_string(),
            selection_reason: "retrieval candidate with no specific context".to_string(),
            budget_impact_tokens: None,
            priority: 50,
            selectable: true,
            dropped_reason: "not selected; unrecognized retrieval tool".to_string(),
        },
    }
}
