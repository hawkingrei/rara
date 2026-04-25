use crate::agent::{CompactState, Message, PlanStepStatus};
use crate::context::{
    CompactionContextView, CompactionSourceContextEntry, ContextBudgetView, PlanContextView,
    PromptContextView, RetrievalContextView, RetrievalSelectedItemContextEntry,
    RetrievalSourceContextEntry, SharedRuntimeContext,
};
use crate::llm::{ContextBudget, LlmBackend};
use crate::prompt::{self, EffectivePrompt, PromptMode, PromptRuntimeConfig, PromptSource};
use crate::workspace::WorkspaceMemory;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssembledContext {
    pub effective_prompt: EffectivePrompt,
    pub compact_instruction: String,
}

#[derive(Debug, Clone)]
pub struct RuntimeContextInputs<'a> {
    pub cwd: String,
    pub branch: String,
    pub session_id: String,
    pub history_len: usize,
    pub total_input_tokens: u32,
    pub total_output_tokens: u32,
    pub execution_mode: String,
    pub plan_steps: Vec<(PlanStepStatus, String)>,
    pub plan_explanation: Option<String>,
    pub compact_state: CompactState,
    pub history: &'a [Message],
    pub vdb_uri: &'a str,
}

impl AssembledContext {
    pub fn system_prompt(&self) -> &str {
        &self.effective_prompt.text
    }
}

#[derive(Clone, Copy)]
pub struct ContextAssembler<'a> {
    workspace: &'a WorkspaceMemory,
    runtime: &'a PromptRuntimeConfig,
}

impl<'a> ContextAssembler<'a> {
    pub fn new(workspace: &'a WorkspaceMemory, runtime: &'a PromptRuntimeConfig) -> Self {
        Self { workspace, runtime }
    }

    pub fn assemble(&self, mode: PromptMode) -> AssembledContext {
        AssembledContext {
            effective_prompt: prompt::build_effective_prompt(self.workspace, self.runtime, mode),
            compact_instruction: prompt::build_compact_instruction(self.runtime),
        }
    }

    pub fn effective_prompt(&self, mode: PromptMode) -> EffectivePrompt {
        self.assemble(mode).effective_prompt
    }

    pub fn system_prompt(&self, mode: PromptMode) -> String {
        self.assemble(mode).effective_prompt.text
    }

    pub fn compact_instruction(&self) -> String {
        prompt::build_compact_instruction(self.runtime)
    }

    pub fn assemble_runtime(
        &self,
        mode: PromptMode,
        inputs: RuntimeContextInputs<'_>,
    ) -> SharedRuntimeContext {
        let effective_prompt = self.effective_prompt(mode);
        let retrieval = RetrievalContextView {
            entries: retrieval_source_entries(
                self.workspace,
                effective_prompt.sources.as_slice(),
                inputs.history,
                inputs.session_id.as_str(),
                inputs.vdb_uri,
            ),
            selected_items: retrieval_selected_items(
                effective_prompt.sources.as_slice(),
                inputs.history,
            ),
        };
        let mut compaction = CompactionContextView::from_compact_state(&inputs.compact_state);
        compaction.source_entries = compaction_source_entries(inputs.history);

        SharedRuntimeContext {
            cwd: inputs.cwd,
            branch: inputs.branch,
            session_id: inputs.session_id,
            history_len: inputs.history_len,
            total_input_tokens: inputs.total_input_tokens,
            total_output_tokens: inputs.total_output_tokens,
            budget: ContextBudgetView::from_compact_state(&inputs.compact_state),
            prompt: PromptContextView::from_effective_prompt(
                effective_prompt,
                self.runtime.append_system_prompt.clone(),
                self.runtime.warnings.clone(),
            ),
            plan: PlanContextView::from_agent_state(
                inputs.execution_mode.as_str(),
                inputs.plan_steps.into_iter(),
                inputs.plan_explanation,
            ),
            compaction,
            retrieval,
        }
    }

    pub fn budget_for(
        &self,
        backend: &dyn LlmBackend,
        history: &[Message],
        tools: &[Value],
    ) -> Option<ContextBudget> {
        backend.context_budget(history, tools)
    }
}

fn retrieval_source_entries(
    workspace: &WorkspaceMemory,
    prompt_sources: &[PromptSource],
    history: &[Message],
    session_id: &str,
    vdb_uri: &str,
) -> Vec<RetrievalSourceContextEntry> {
    let workspace_memory_active = prompt_sources
        .iter()
        .any(|source| source.kind_label() == "local_memory");
    let workspace_memory_path = workspace.rara_dir.join("memory.md");
    let workspace_memory_exists = workspace.has_memory_file_cached();
    let workspace_memory_status = if workspace_memory_active {
        "active"
    } else if workspace_memory_exists {
        "available"
    } else {
        "missing"
    };
    let thread_history_status = if history.is_empty() {
        "empty"
    } else {
        "available"
    };
    let vector_memory_status = if vdb_uri.is_empty() {
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
            detail: format!("session={} messages={}", session_id, history.len()),
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
    ]
}

fn retrieval_selected_items(
    prompt_sources: &[PromptSource],
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
    prompt_sources: &[PromptSource],
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

fn compaction_source_entries(history: &[Message]) -> Vec<CompactionSourceContextEntry> {
    let mut entries = Vec::new();
    let mut compact_boundary_seen = false;

    for message in history {
        let Some(items) = message.content.as_array() else {
            continue;
        };
        for item in items {
            let Some(item_type) = item.get("type").and_then(Value::as_str) else {
                continue;
            };
            match item_type {
                "compacted_summary" => entries.push(CompactionSourceContextEntry {
                    order: 0,
                    kind: "compacted_summary".to_string(),
                    label: "Compacted Summary".to_string(),
                    detail: summarize_text_block(item.get("text").and_then(Value::as_str)),
                    inclusion_reason: "carried forward because the conversation history was compacted into a summary block".to_string(),
                }),
                "recent_files" => entries.push(CompactionSourceContextEntry {
                    order: 0,
                    kind: "recent_files".to_string(),
                    label: "Recent Files".to_string(),
                    detail: summarize_recent_files(item.get("files").and_then(Value::as_array)),
                    inclusion_reason: "carried forward so the next turn keeps a lightweight view of recently touched files".to_string(),
                }),
                "recent_file_excerpts" => entries.push(CompactionSourceContextEntry {
                    order: 0,
                    kind: "recent_file_excerpts".to_string(),
                    label: "Recent File Excerpts".to_string(),
                    detail: summarize_recent_file_excerpts(item.get("files").and_then(Value::as_array)),
                    inclusion_reason: "carried forward so the next turn retains short excerpts from recently referenced files".to_string(),
                }),
                "compact_boundary" if !compact_boundary_seen => {
                    compact_boundary_seen = true;
                    entries.push(CompactionSourceContextEntry {
                        order: 0,
                        kind: "compact_boundary".to_string(),
                        label: "Compaction Boundary".to_string(),
                        detail: summarize_compact_boundary(item),
                        inclusion_reason: "recorded to explain where the latest compaction boundary cut the thread history".to_string(),
                    });
                }
                _ => {}
            }
        }
    }

    for (idx, entry) in entries.iter_mut().enumerate() {
        entry.order = idx + 1;
    }
    entries
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

fn summarize_compact_boundary(item: &Value) -> String {
    let version = item
        .get("version")
        .and_then(Value::as_u64)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string());
    let before_tokens = item
        .get("before_tokens")
        .and_then(Value::as_u64)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string());
    let recent_file_count = item
        .get("recent_file_count")
        .and_then(Value::as_u64)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string());
    format!("version={version} before_tokens={before_tokens} recent_files={recent_file_count}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{ContentBlock, LlmResponse};
    use anyhow::Result;
    use async_trait::async_trait;
    use rara_config::RaraConfig;
    use serde_json::json;
    use std::path::PathBuf;

    struct BudgetBackend {
        budget: Option<ContextBudget>,
    }

    #[async_trait]
    impl LlmBackend for BudgetBackend {
        async fn ask(&self, _messages: &[Message], _tools: &[Value]) -> Result<LlmResponse> {
            Ok(LlmResponse {
                content: vec![ContentBlock::Text {
                    text: "ok".to_string(),
                }],
                stop_reason: Some("end_turn".to_string()),
                usage: None,
            })
        }

        async fn embed(&self, _text: &str) -> Result<Vec<f32>> {
            Ok(vec![0.0; 8])
        }

        async fn summarize(&self, _messages: &[Message], _instruction: &str) -> Result<String> {
            Ok("summary".to_string())
        }

        fn context_budget(&self, _messages: &[Message], _tools: &[Value]) -> Option<ContextBudget> {
            self.budget
        }
    }

    fn test_workspace() -> WorkspaceMemory {
        WorkspaceMemory::from_paths(PathBuf::from("/repo"), PathBuf::from("/repo/.rara"))
    }

    #[test]
    fn assemble_keeps_prompt_and_compact_instruction_together() {
        let workspace = test_workspace();
        let runtime = PromptRuntimeConfig {
            append_system_prompt: Some("appendix".to_string()),
            compact_prompt: Some("compact me".to_string()),
            ..PromptRuntimeConfig::default()
        };

        let assembled = ContextAssembler::new(&workspace, &runtime).assemble(PromptMode::Plan);

        assert!(assembled.system_prompt().contains("appendix"));
        assert_eq!(assembled.compact_instruction, "compact me");
        assert!(assembled
            .effective_prompt
            .section_keys
            .contains(&"append_system_prompt"));
    }

    #[test]
    fn assemble_runtime_collects_budget_and_runtime_views() {
        let workspace = test_workspace();
        let runtime = PromptRuntimeConfig {
            append_system_prompt: Some("appendix".to_string()),
            warnings: vec!["missing prompt file".to_string()],
            ..PromptRuntimeConfig::default()
        };

        let history = vec![Message {
            role: "assistant".to_string(),
            content: json!([{"type":"compacted_summary","text":"summary"}]),
        }];

        let runtime_context = ContextAssembler::new(&workspace, &runtime).assemble_runtime(
            PromptMode::Plan,
            RuntimeContextInputs {
                cwd: "repo".to_string(),
                branch: "main".to_string(),
                session_id: "session-1".to_string(),
                history_len: 3,
                total_input_tokens: 11,
                total_output_tokens: 7,
                execution_mode: "plan".to_string(),
                plan_steps: vec![(PlanStepStatus::Pending, "inspect bootstrap".to_string())],
                plan_explanation: Some("Keep one assembly path.".to_string()),
                compact_state: crate::agent::CompactState {
                    estimated_history_tokens: 1234,
                    context_window_tokens: Some(8192),
                    compact_threshold_tokens: 7000,
                    reserved_output_tokens: 1024,
                    ..Default::default()
                },
                history: &history,
                vdb_uri: "memory://vdb",
            },
        );

        assert_eq!(runtime_context.session_id, "session-1");
        assert_eq!(runtime_context.budget.context_window_tokens, Some(8192));
        assert_eq!(runtime_context.budget.compact_threshold_tokens, 7000);
        assert_eq!(runtime_context.plan.execution_mode, "plan");
        assert_eq!(runtime_context.plan.steps.len(), 1);
        assert_eq!(
            runtime_context.prompt.warnings,
            vec!["missing prompt file".to_string()]
        );
        assert_eq!(
            runtime_context.prompt.append_system_prompt.as_deref(),
            Some("appendix")
        );
        assert_eq!(runtime_context.retrieval.entries.len(), 3);
        assert_eq!(runtime_context.compaction.source_entries.len(), 1);
    }

    #[test]
    fn budget_for_passthrough_uses_backend_budget() {
        let workspace = test_workspace();
        let runtime = PromptRuntimeConfig::from_config(&RaraConfig::default());
        let budget = ContextBudget {
            context_window_tokens: 200_000,
            reserved_output_tokens: 4_096,
            compact_threshold_tokens: 190_000,
        };
        let backend = BudgetBackend {
            budget: Some(budget),
        };

        let result = ContextAssembler::new(&workspace, &runtime).budget_for(
            &backend,
            &[Message {
                role: "user".to_string(),
                content: json!([{"type":"text","text":"hello"}]),
            }],
            &[json!({"name":"read_file"})],
        );

        assert_eq!(result, Some(budget));
    }
}
