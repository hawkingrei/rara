use super::*;
use crate::context::{
    CompactionContextView, CompactionSourceContextEntry, PlanContextView, PromptContextView,
    RetrievalContextView, RetrievalSelectedItemContextEntry, RetrievalSourceContextEntry,
    SharedRuntimeContext,
};
use serde_json::Value;
use std::collections::HashMap;

impl Agent {
    pub fn shared_runtime_context(&self) -> SharedRuntimeContext {
        let (cwd, branch) = self.workspace.get_env_info();
        let effective_prompt = self.effective_prompt();

        SharedRuntimeContext {
            cwd,
            branch,
            session_id: self.session_id.clone(),
            history_len: self.history.len(),
            total_input_tokens: self.total_input_tokens,
            total_output_tokens: self.total_output_tokens,
            prompt: PromptContextView::from_effective_prompt(
                effective_prompt,
                self.prompt_config().append_system_prompt.clone(),
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
                source_entries: compaction_source_entries(&self.history),
            },
            retrieval: RetrievalContextView {
                entries: retrieval_source_entries(self),
                selected_items: retrieval_selected_items(self),
            },
        }
    }
}

fn retrieval_source_entries(agent: &Agent) -> Vec<RetrievalSourceContextEntry> {
    let prompt_sources = agent.effective_prompt().sources;
    let workspace_memory_active = prompt_sources
        .iter()
        .any(|source| source.kind_label() == "local_memory");
    let workspace_memory_path = agent.workspace.rara_dir.join("memory.md");
    let workspace_memory_exists = workspace_memory_path.exists();
    let workspace_memory_status = if workspace_memory_active {
        "active"
    } else if workspace_memory_exists {
        "available"
    } else {
        "missing"
    };

    let thread_history_status = if agent.history.is_empty() {
        "empty"
    } else {
        "available"
    };

    let vector_memory_status = if agent.vdb.uri().is_empty() {
        "missing"
    } else {
        "available"
    };

    vec![
        RetrievalSourceContextEntry {
            order: 1,
            kind: "workspace_memory".to_string(),
            label: "Workspace Memory".to_string(),
            status: workspace_memory_status.to_string(),
            detail: workspace_memory_path.display().to_string(),
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
            detail: format!(
                "session={} messages={}",
                agent.session_id,
                agent.history.len()
            ),
            inclusion_reason: if agent.history.is_empty() {
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
            detail: agent.vdb.uri().to_string(),
            inclusion_reason: if vector_memory_status == "available" {
                "configured as the durable vector-backed memory store for later retrieval, even though the current recall path is still limited".to_string()
            } else {
                "no vector-backed memory store is configured for retrieval".to_string()
            },
        },
    ]
}

fn retrieval_selected_items(agent: &Agent) -> Vec<RetrievalSelectedItemContextEntry> {
    let mut items = Vec::new();
    let prompt_sources = agent.effective_prompt().sources;

    items.extend(workspace_memory_selected_items(&prompt_sources));
    items.extend(compacted_history_selected_items(&agent.history));
    items.extend(retrieval_tool_selected_items(&agent.history));

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
        .filter(|source| source.kind_label() == "local_memory")
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
    compaction_source_entries(history)
        .into_iter()
        .filter(|entry| entry.kind != "compact_boundary")
        .map(|entry| RetrievalSelectedItemContextEntry {
            order: 0,
            kind: entry.kind,
            label: entry.label,
            detail: entry.detail,
            inclusion_reason: entry.inclusion_reason,
        })
        .collect()
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
        if item.get("type").and_then(Value::as_str) != Some("tool_use") {
            continue;
        }
        let Some(name) = item.get("name").and_then(Value::as_str) else {
            continue;
        };
        if name != "retrieve_experience" && name != "retrieve_session_context" {
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
    for item in blocks {
        if item.get("type").and_then(Value::as_str) != Some("tool_result") {
            continue;
        }
        let Some(tool_use_id) = item.get("tool_use_id").and_then(Value::as_str) else {
            continue;
        };
        let Some((tool_name, query)) = pending.remove(tool_use_id) else {
            continue;
        };
        let Some(content) = item.get("content").and_then(Value::as_str) else {
            continue;
        };
        let Some(payload) = parse_tool_result_payload(content) else {
            continue;
        };
        if let Some(selected_item) =
            retrieval_selected_item_from_payload(&tool_name, query.as_deref(), &payload)
        {
            items.push(selected_item);
        }
    }
}

fn parse_tool_result_payload(content: &str) -> Option<Value> {
    let trimmed = content.trim();
    if let Ok(value) = serde_json::from_str(trimmed) {
        return Some(value);
    }
    let payload = trimmed.split_once("\nPayload:\n")?.1.trim();
    serde_json::from_str(payload).ok()
}

fn retrieval_selected_item_from_payload(
    tool_name: &str,
    query: Option<&str>,
    payload: &Value,
) -> Option<RetrievalSelectedItemContextEntry> {
    match tool_name {
        "retrieve_experience" => retrieve_experience_selected_item(query, payload),
        "retrieve_session_context" => retrieve_session_context_selected_item(query, payload),
        _ => None,
    }
}

fn retrieve_experience_selected_item(
    query: Option<&str>,
    payload: &Value,
) -> Option<RetrievalSelectedItemContextEntry> {
    let experiences = payload.get("relevant_experiences")?.as_array()?;
    if experiences.is_empty() {
        return None;
    }
    let preview = experiences
        .iter()
        .take(2)
        .map(render_selected_value_preview)
        .collect::<Vec<_>>()
        .join(" | ");
    let query = query.unwrap_or("-");
    Some(RetrievalSelectedItemContextEntry {
        order: 0,
        kind: "retrieved_workspace_memory".to_string(),
        label: "Retrieved Experience".to_string(),
        detail: format!(
            "query={query}; recalled={} item(s); preview: {}",
            experiences.len(),
            if preview.is_empty() { "-".to_string() } else { preview }
        ),
        inclusion_reason: "selected because the retrieval tool returned relevant durable memory candidates for the current task".to_string(),
    })
}

fn retrieve_session_context_selected_item(
    query: Option<&str>,
    payload: &Value,
) -> Option<RetrievalSelectedItemContextEntry> {
    if payload.get("status").and_then(Value::as_str) == Some("no_context_found") {
        return None;
    }
    let summary = payload
        .get("summary")
        .and_then(Value::as_str)
        .or_else(|| payload.get("context").and_then(Value::as_str))
        .map(str::to_string)
        .or_else(|| {
            payload
                .get("results")
                .and_then(Value::as_array)
                .filter(|results| !results.is_empty())
                .map(|results| {
                    results
                        .iter()
                        .take(2)
                        .map(render_selected_value_preview)
                        .collect::<Vec<_>>()
                        .join(" | ")
                })
        })?;
    let query = query.unwrap_or("-");
    Some(RetrievalSelectedItemContextEntry {
        order: 0,
        kind: "retrieved_thread_memory".to_string(),
        label: "Retrieved Session Context".to_string(),
        detail: format!("query={query}; summary: {summary}"),
        inclusion_reason: "selected because the session-context retrieval tool returned prior thread history that is still relevant to the current turn".to_string(),
    })
}

fn render_selected_value_preview(value: &Value) -> String {
    if let Some(text) = value.as_str() {
        return text.trim().to_string();
    }
    if let Some(summary) = value.get("summary").and_then(Value::as_str) {
        return summary.trim().to_string();
    }
    if let Some(text) = value.get("text").and_then(Value::as_str) {
        return text.trim().to_string();
    }
    value.to_string()
}

fn summarize_workspace_memory_source(content: &str) -> String {
    let line_count = content.lines().count();
    let first_line = content
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("-");
    format!("{line_count} line(s); first line: {first_line}")
}

fn compaction_source_entries(history: &[Message]) -> Vec<CompactionSourceContextEntry> {
    history
        .iter()
        .filter_map(compaction_source_entry_from_message)
        .enumerate()
        .map(|(idx, mut entry)| {
            entry.order = idx + 1;
            entry
        })
        .collect()
}

fn compaction_source_entry_from_message(message: &Message) -> Option<CompactionSourceContextEntry> {
    if message.role != "system" {
        return None;
    }
    if let Some(boundary) = compact_boundary_entry(message) {
        return Some(boundary);
    }
    let text = message.content.as_str()?;
    if text.starts_with("STRUCTURED SUMMARY OF PREVIOUS CONVERSATION:\n") {
        let summary = text
            .trim_start_matches("STRUCTURED SUMMARY OF PREVIOUS CONVERSATION:\n")
            .lines()
            .next()
            .unwrap_or("-")
            .trim()
            .to_string();
        return Some(CompactionSourceContextEntry {
            order: 0,
            kind: "compacted_summary".to_string(),
            label: "Compacted Thread Summary".to_string(),
            detail: if summary.is_empty() {
                "-".to_string()
            } else {
                summary
            },
            inclusion_reason: "included because older thread history was compacted into a structured summary instead of being replayed verbatim".to_string(),
        });
    }
    if text.starts_with("RECENT FILES FROM COMPACTED HISTORY:\n") {
        let count = text.lines().skip(1).filter(|line| line.starts_with("- ")).count();
        return Some(CompactionSourceContextEntry {
            order: 0,
            kind: "recent_files".to_string(),
            label: "Recent Files From Compacted History".to_string(),
            detail: format!("{count} carried-over file path(s)"),
            inclusion_reason: "included to preserve recently inspected or edited file paths after older history was compacted".to_string(),
        });
    }
    if text.starts_with("RECENT FILE EXCERPTS FROM COMPACTED HISTORY:\n") {
        let excerpt_count = text.matches("### ").count();
        return Some(CompactionSourceContextEntry {
            order: 0,
            kind: "recent_file_excerpts".to_string(),
            label: "Recent File Excerpts From Compacted History".to_string(),
            detail: format!("{excerpt_count} excerpt block(s)"),
            inclusion_reason: "included to keep small source snippets from recently read files available after compaction".to_string(),
        });
    }
    None
}

fn compact_boundary_entry(message: &Message) -> Option<CompactionSourceContextEntry> {
    let content = message.content.as_object()?;
    if content.get("type").and_then(Value::as_str) != Some("compact_boundary") {
        return None;
    }
    let before_tokens = content
        .get("before_tokens")
        .and_then(Value::as_u64)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string());
    let recent_file_count = content
        .get("recent_file_count")
        .and_then(Value::as_u64)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string());
    Some(CompactionSourceContextEntry {
        order: 0,
        kind: "compact_boundary".to_string(),
        label: "Compaction Boundary".to_string(),
        detail: format!("before_tokens={before_tokens}, recent_file_count={recent_file_count}"),
        inclusion_reason: "included as the structured boundary marker that explains where older history was compacted".to_string(),
    })
}
