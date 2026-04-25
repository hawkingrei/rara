use super::*;
use crate::context::{
    CompactionContextView, PlanContextView, PromptContextView, RetrievalContextView,
    RetrievalSelectedItemContextEntry, SharedRuntimeContext,
};
use crate::prompt::PromptSourceKind;
use serde_json::Value;
use std::collections::HashMap;

impl Agent {
    pub fn shared_runtime_context(&self) -> SharedRuntimeContext {
        let (cwd, branch) = self.workspace.get_env_info();
        let effective_prompt = self.effective_prompt();
        let remaining_input_budget_tokens = self
            .compact_state
            .context_window_tokens
            .map(|window| {
                window.saturating_sub(
                    self.compact_state.estimated_history_tokens
                        + self.compact_state.reserved_output_tokens,
                )
            });

        SharedRuntimeContext {
            cwd,
            branch,
            session_id: self.session_id.clone(),
            history_len: self.history.len(),
            total_input_tokens: self.total_input_tokens,
            total_output_tokens: self.total_output_tokens,
            prompt: PromptContextView::from_effective_prompt(
                effective_prompt.clone(),
                self.prompt_config().warnings.clone(),
            ),
            plan: PlanContextView::from_agent_state(
                self.execution_mode_label(),
                self.current_plan
                    .iter()
                    .map(|step| (step.status.clone(), step.step.clone())),
                self.plan_explanation.clone(),
            ),
            compaction: CompactionContextView {
                estimated_history_tokens: self.compact_state.estimated_history_tokens,
                context_window_tokens: self.compact_state.context_window_tokens,
                compact_threshold_tokens: self.compact_state.compact_threshold_tokens,
                reserved_output_tokens: self.compact_state.reserved_output_tokens,
                compaction_count: self.compact_state.compaction_count,
                last_compaction_before_tokens: self.compact_state.last_compaction_before_tokens,
                last_compaction_after_tokens: self.compact_state.last_compaction_after_tokens,
                last_compaction_recent_files: self.compact_state.last_compaction_recent_files.clone(),
                last_compaction_boundary_version: self
                    .compact_state
                    .last_compaction_boundary
                    .map(|boundary| boundary.version),
                last_compaction_boundary_before_tokens: self
                    .compact_state
                    .last_compaction_boundary
                    .map(|boundary| boundary.before_tokens),
                last_compaction_boundary_recent_file_count: self
                    .compact_state
                    .last_compaction_boundary
                    .map(|boundary| boundary.recent_file_count),
            },
            retrieval: RetrievalContextView {
                remaining_input_budget_tokens,
                selected_items: retrieval_selected_items(
                    effective_prompt.sources.as_slice(),
                    self.history.as_slice(),
                ),
            },
        }
    }
}

fn retrieval_selected_items(
    prompt_sources: &[crate::prompt::PromptSource],
    history: &[Message],
) -> Vec<RetrievalSelectedItemContextEntry> {
    let mut items = Vec::new();
    items.extend(workspace_memory_selected_items(prompt_sources));
    items.extend(compacted_history_selected_items(history));
    items.extend(retrieval_tool_selected_items(history));
    for (idx, item) in items.iter_mut().enumerate() {
        item.order = idx + 1;
    }
    items
}

fn workspace_memory_selected_items(
    prompt_sources: &[crate::prompt::PromptSource],
) -> Vec<RetrievalSelectedItemContextEntry> {
    prompt_sources
        .iter()
        .filter(|source| matches!(source.kind, PromptSourceKind::LocalMemory))
        .map(|source| RetrievalSelectedItemContextEntry {
            order: 0,
            kind: "workspace_memory".to_string(),
            label: "Workspace Memory".to_string(),
            detail: format!(
                "{}; {}",
                source.display_path,
                summarize_workspace_memory_source(source.content.as_str())
            ),
            inclusion_reason: "selected because the current effective prompt includes the workspace memory file as an active input".to_string(),
        })
        .collect()
}

fn compacted_history_selected_items(history: &[Message]) -> Vec<RetrievalSelectedItemContextEntry> {
    let mut items = Vec::new();

    for message in history {
        let Some(blocks) = message.content.as_array() else {
            continue;
        };
        for block in blocks {
            let Some(kind) = block.get("type").and_then(Value::as_str) else {
                continue;
            };
            match kind {
                "compacted_summary" => items.push(RetrievalSelectedItemContextEntry {
                    order: 0,
                    kind: "compacted_summary".to_string(),
                    label: "Compacted Summary".to_string(),
                    detail: summarize_text_block(block.get("text").and_then(Value::as_str)),
                    inclusion_reason: "selected because compacted history summary is still part of the active thread context".to_string(),
                }),
                "recent_files" => items.push(RetrievalSelectedItemContextEntry {
                    order: 0,
                    kind: "recent_files".to_string(),
                    label: "Recent Files".to_string(),
                    detail: summarize_recent_files(block.get("files").and_then(Value::as_array)),
                    inclusion_reason: "selected because recent file carry-over survived compaction and still consumes retrieval budget".to_string(),
                }),
                "recent_file_excerpts" => items.push(RetrievalSelectedItemContextEntry {
                    order: 0,
                    kind: "recent_file_excerpts".to_string(),
                    label: "Recent File Excerpts".to_string(),
                    detail: summarize_recent_file_excerpts(
                        block.get("files").and_then(Value::as_array),
                    ),
                    inclusion_reason: "selected because file excerpts from compacted history are still active in the current thread context".to_string(),
                }),
                _ => {}
            }
        }
    }

    items
}

fn retrieval_tool_selected_items(history: &[Message]) -> Vec<RetrievalSelectedItemContextEntry> {
    let mut pending = HashMap::new();
    let mut items = Vec::new();

    for message in history {
        match message.role.as_str() {
            "assistant" => collect_pending_retrieval_tool_uses(&mut pending, message),
            "user" => collect_retrieval_tool_results(&mut pending, &mut items, message),
            _ => {}
        }
    }

    items
}

fn collect_pending_retrieval_tool_uses(
    pending: &mut HashMap<String, (String, Option<String>)>,
    message: &Message,
) {
    let Some(items) = message.content.as_array() else {
        return;
    };
    for item in items {
        let Some(item_type) = item.get("type").and_then(Value::as_str) else {
            continue;
        };
        if item_type != "tool_use" {
            continue;
        }
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
    items: &mut Vec<RetrievalSelectedItemContextEntry>,
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
        let detail = query
            .map(|query| format!("query={query}"))
            .unwrap_or_else(|| "query unavailable".to_string());
        items.push(RetrievalSelectedItemContextEntry {
            order: 0,
            kind: name.clone(),
            label: match name.as_str() {
                "retrieve_experience" => "Retrieved Experience".to_string(),
                "retrieve_session_context" => "Retrieved Session Context".to_string(),
                _ => name,
            },
            detail,
            inclusion_reason: "selected because a retrieval tool result was returned in the current thread history".to_string(),
        });
    }
}

fn summarize_workspace_memory_source(content: &str) -> String {
    let line_count = content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
    match line_count {
        0 => "empty".to_string(),
        1 => "1 non-empty line".to_string(),
        _ => format!("{line_count} non-empty lines"),
    }
}

fn summarize_text_block(text: Option<&str>) -> String {
    text.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            let condensed = value.split_whitespace().collect::<Vec<_>>().join(" ");
            if condensed.len() > 96 {
                format!("{}...", &condensed[..93])
            } else {
                condensed
            }
        })
        .unwrap_or_else(|| "no summary text".to_string())
}

fn summarize_recent_files(files: Option<&Vec<Value>>) -> String {
    let files = files
        .into_iter()
        .flat_map(|items| items.iter())
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    match files.len() {
        0 => "no files".to_string(),
        1 => files[0].to_string(),
        _ => format!("{} (+{} more)", files[0], files.len() - 1),
    }
}

fn summarize_recent_file_excerpts(files: Option<&Vec<Value>>) -> String {
    let count = files.into_iter().flat_map(|items| items.iter()).count();
    match count {
        0 => "no excerpts".to_string(),
        1 => "1 excerpt".to_string(),
        _ => format!("{count} excerpts"),
    }
}
