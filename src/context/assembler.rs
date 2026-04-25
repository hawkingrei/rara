use crate::agent::{CompactState, Message, PlanStepStatus};
use crate::context::{
    CompactionContextView, CompactionSourceContextEntry, ContextAssemblyEntry, ContextAssemblyView,
    ContextBudgetView, MemorySelectionContextView, MemorySelectionItemContextEntry,
    PlanContextView, PromptContextView, RetrievalContextView, RetrievalSourceContextEntry,
    SharedRuntimeContext,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssembledTurnContext {
    pub prompt: AssembledContext,
    pub runtime: SharedRuntimeContext,
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
    pub pending_interactions: Vec<RuntimeInteractionInput>,
}

#[derive(Debug, Clone)]
pub struct RuntimeInteractionInput {
    pub kind: String,
    pub title: String,
    pub summary: String,
    pub source: Option<String>,
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

    pub fn assemble_turn(
        &self,
        mode: PromptMode,
        inputs: RuntimeContextInputs<'_>,
    ) -> AssembledTurnContext {
        let prompt = self.assemble(mode);
        let runtime =
            self.assemble_runtime_from_effective_prompt(prompt.effective_prompt.clone(), inputs);
        AssembledTurnContext { prompt, runtime }
    }

    pub fn assemble_runtime(
        &self,
        mode: PromptMode,
        inputs: RuntimeContextInputs<'_>,
    ) -> SharedRuntimeContext {
        let effective_prompt = self.effective_prompt(mode);
        self.assemble_runtime_from_effective_prompt(effective_prompt, inputs)
    }

    fn assemble_runtime_from_effective_prompt(
        &self,
        effective_prompt: EffectivePrompt,
        inputs: RuntimeContextInputs<'_>,
    ) -> SharedRuntimeContext {
        let stable_instructions_budget = estimate_text_tokens(effective_prompt.text.as_str());
        let workspace_prompt_budget = effective_prompt
            .sources
            .iter()
            .filter(|source| {
                matches!(
                    source.kind_label(),
                    "project_instruction" | "local_instruction" | "local_memory"
                )
            })
            .map(|source| estimate_text_tokens(source.content.as_str()))
            .sum();
        let retrieval_entries = retrieval_source_entries(
            self.workspace,
            effective_prompt.sources.as_slice(),
            inputs.history,
            inputs.session_id.as_str(),
            inputs.vdb_uri,
        );
        let mut compaction = CompactionContextView::from_compact_state(&inputs.compact_state);
        compaction.source_entries = compaction_source_entries(inputs.history);
        let compacted_history_budget = compaction
            .source_entries
            .iter()
            .map(|entry| estimate_text_tokens(entry.detail.as_str()))
            .sum();
        let active_turn_budget = active_turn_budget(
            inputs.plan_explanation.as_deref(),
            inputs.plan_steps.as_slice(),
            inputs.pending_interactions.as_slice(),
            inputs.history,
        );
        let selection_budget = inputs.compact_state.context_window_tokens.map(|window| {
            window
                .saturating_sub(inputs.compact_state.reserved_output_tokens)
                .saturating_sub(stable_instructions_budget)
                .saturating_sub(workspace_prompt_budget)
                .saturating_sub(active_turn_budget)
                .saturating_sub(compacted_history_budget)
        });
        let retrieval = RetrievalContextView {
            entries: retrieval_entries,
            memory_selection: memory_selection(
                effective_prompt.sources.as_slice(),
                inputs.history,
                inputs.session_id.as_str(),
                inputs.vdb_uri,
                selection_budget,
            ),
        };
        let retrieved_memory_budget = retrieval
            .memory_selection
            .selected_items
            .iter()
            .filter(|item| {
                matches!(
                    item.kind.as_str(),
                    "retrieved_workspace_memory" | "retrieved_thread_context"
                )
            })
            .map(|item| item.budget_impact_tokens.unwrap_or_default())
            .sum();
        let assembly = assemble_context_view(
            &effective_prompt,
            inputs.plan_explanation.as_deref(),
            inputs.plan_steps.as_slice(),
            inputs.pending_interactions.as_slice(),
            compaction.source_entries.as_slice(),
            retrieval.memory_selection.selected_items.as_slice(),
            retrieval.memory_selection.dropped_items.as_slice(),
            inputs.history,
        );

        SharedRuntimeContext {
            cwd: inputs.cwd,
            branch: inputs.branch,
            session_id: inputs.session_id,
            history_len: inputs.history_len,
            total_input_tokens: inputs.total_input_tokens,
            total_output_tokens: inputs.total_output_tokens,
            budget: ContextBudgetView::from_compact_state(
                &inputs.compact_state,
                stable_instructions_budget,
                workspace_prompt_budget,
                active_turn_budget,
                compacted_history_budget,
                retrieved_memory_budget,
            ),
            assembly,
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

fn assemble_context_view(
    effective_prompt: &EffectivePrompt,
    plan_explanation: Option<&str>,
    plan_steps: &[(PlanStepStatus, String)],
    pending_interactions: &[RuntimeInteractionInput],
    compaction_entries: &[CompactionSourceContextEntry],
    selected_memory_items: &[MemorySelectionItemContextEntry],
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
            "project_instruction" | "local_instruction" => "stable_instructions",
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
                "included because structured plan steps are part of the current active thread state"
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
            inclusion_reason: "included because a pending interaction must survive restore and remain visible to the active runtime".to_string(),
            budget_impact_tokens: Some(estimate_text_tokens(interaction.summary.as_str())),
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

    for tool_result in latest_tool_results(history) {
        push(ContextAssemblyEntry {
            order: 0,
            layer: "active_turn_state".to_string(),
            kind: "tool_result".to_string(),
            label: tool_result.0,
            source_path: None,
            injected: true,
            inclusion_reason: "included because recent tool results are part of the current working set for the active turn".to_string(),
            budget_impact_tokens: Some(estimate_text_tokens(tool_result.1.as_str())),
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

fn active_turn_budget(
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

fn latest_user_request(history: &[Message]) -> Option<String> {
    history
        .iter()
        .rev()
        .find(|message| message.role == "user")
        .and_then(|message| extract_latest_text(message))
}

fn latest_tool_results(history: &[Message]) -> Vec<(String, String)> {
    history
        .iter()
        .rev()
        .find(|message| message.role == "user" && message.content.as_array().is_some())
        .and_then(|message| message.content.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    (item.get("type").and_then(Value::as_str) == Some("tool_result")).then(|| {
                        let tool_id = item
                            .get("tool_use_id")
                            .and_then(Value::as_str)
                            .unwrap_or("tool_result")
                            .to_string();
                        let content = item
                            .get("content")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        (format!("Tool Result {tool_id}"), content)
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn extract_latest_text(message: &Message) -> Option<String> {
    if let Some(text) = message.content.as_str() {
        let trimmed = text.trim();
        return (!trimmed.is_empty()).then(|| trimmed.to_string());
    }
    message.content.as_array().and_then(|items| {
        items
            .iter()
            .rev()
            .find_map(|item| {
                (item.get("type").and_then(Value::as_str) == Some("text"))
                    .then(|| item.get("text").and_then(Value::as_str))
                    .flatten()
            })
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
    })
}

fn estimate_text_tokens(text: &str) -> usize {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        0
    } else {
        // Use a conservative text-to-token estimate so local/smaller-context
        // models do not silently overrun the remaining input budget.
        trimmed.len().div_ceil(3)
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

fn memory_selection(
    prompt_sources: &[PromptSource],
    history: &[Message],
    session_id: &str,
    vdb_uri: &str,
    selection_budget_tokens: Option<usize>,
) -> MemorySelectionContextView {
    let mut selected_items = fixed_memory_selection_items(prompt_sources, history);
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

    let mut dropped_items = discretionary.dropped_items;
    if !selected_items
        .iter()
        .any(|item| item.kind == "workspace_memory")
    {
        dropped_items.push(workspace_memory_dropped_item(prompt_sources));
    }
    for (idx, item) in dropped_items.iter_mut().enumerate() {
        item.order = idx + 1;
    }

    MemorySelectionContextView {
        selection_budget_tokens,
        selected_items,
        dropped_items,
    }
}

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
    dropped_items: Vec<MemorySelectionItemContextEntry>,
}

fn fixed_memory_selection_items(
    prompt_sources: &[PromptSource],
    history: &[Message],
) -> Vec<MemorySelectionItemContextEntry> {
    let mut items = Vec::new();
    items.extend(workspace_memory_selected_items(prompt_sources));
    items.extend(compacted_history_selected_items(history));
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
            summarize_workspace_memory_source(source.content.as_str())
        ),
        selection_reason: "selected because the current effective prompt includes the workspace memory file as an active input".to_string(),
        budget_impact_tokens: Some(estimate_text_tokens(source.content.as_str())),
        dropped_reason: None,
    }
}

fn compacted_history_selected_items(history: &[Message]) -> Vec<MemorySelectionItemContextEntry> {
    compaction_source_entries(history)
        .into_iter()
        .filter(|entry| entry.kind != "compact_boundary")
        .map(|entry| MemorySelectionItemContextEntry {
            order: 0,
            kind: entry.kind,
            label: entry.label,
            budget_impact_tokens: Some(estimate_text_tokens(entry.detail.as_str())),
            detail: entry.detail,
            selection_reason: entry.inclusion_reason,
            dropped_reason: None,
        })
        .collect()
}

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
            (cost > remaining).then(|| {
                format!(
                    "not selected because it would exceed the remaining memory-selection budget ({cost} > {remaining})"
                )
            })
        } else {
            None
        };

        if let Some(dropped_reason) = should_drop {
            decision
                .dropped_items
                .push(MemorySelectionItemContextEntry {
                    order: 0,
                    kind: candidate.kind,
                    label: candidate.label,
                    detail: candidate.detail,
                    selection_reason: candidate.selection_reason,
                    budget_impact_tokens: candidate.budget_impact_tokens,
                    dropped_reason: Some(dropped_reason),
                });
            continue;
        }

        if let (Some(remaining), Some(cost)) =
            (remaining_budget.as_mut(), candidate.budget_impact_tokens)
        {
            *remaining = remaining.saturating_sub(cost);
        }
        selected_kinds.push(candidate.kind.clone());
        decision
            .selected_items
            .push(MemorySelectionItemContextEntry {
                order: 0,
                kind: candidate.kind,
                label: candidate.label,
                detail: candidate.detail,
                selection_reason: candidate.selection_reason,
                budget_impact_tokens: candidate.budget_impact_tokens,
                dropped_reason: None,
            });
    }

    decision
}

fn workspace_memory_dropped_item(
    prompt_sources: &[PromptSource],
) -> MemorySelectionItemContextEntry {
    let workspace_memory_available = prompt_sources
        .iter()
        .any(|source| source.kind_label() == "local_memory");
    MemorySelectionItemContextEntry {
        order: 0,
        kind: "workspace_memory".to_string(),
        label: "Workspace Memory".to_string(),
        detail: if workspace_memory_available {
            "workspace prompt source is available, but not active in the current assembled prompt".to_string()
        } else {
            "no injected workspace memory prompt source".to_string()
        },
        selection_reason: "workspace memory participates in the selection contract even when it is not part of the current assembled working set".to_string(),
        budget_impact_tokens: None,
        dropped_reason: Some(if workspace_memory_available {
            "not selected into the current turn because workspace memory was not activated as a prompt input".to_string()
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
            let experiences = extract_json_array_strings(content, "relevant_experiences");
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
            let summary = extract_json_string_field(content, "summary")
                .unwrap_or_else(|| "no session-context summary".to_string());
            let query = query.unwrap_or("query unavailable");
            let detail = format!("query={query}; summary: {summary}");
            MemorySelectionCandidate {
                kind: "retrieved_thread_context".to_string(),
                label: "Retrieved Session Context".to_string(),
                budget_impact_tokens: Some(estimate_text_tokens(detail.as_str())),
                detail,
                selection_reason: "selected because the retrieval tool returned focused thread-context material for the current task".to_string(),
                priority: 20,
                selectable: true,
                dropped_reason: "not selected after ranking the retrieved thread-context candidate against the current memory-selection budget".to_string(),
            }
        }
        other => MemorySelectionCandidate {
            kind: other.to_string(),
            label: other.to_string(),
            detail: query
                .map(|query| format!("query={query}"))
                .unwrap_or_else(|| "query unavailable".to_string()),
            selection_reason: "selected because a retrieval tool result was returned in the current thread history".to_string(),
            budget_impact_tokens: Some(estimate_text_tokens(content)),
            priority: 50,
            selectable: true,
            dropped_reason: "not selected after ranking the retrieval candidate against the current memory-selection budget".to_string(),
        },
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

fn extract_json_array_strings(content: &str, key: &str) -> Vec<String> {
    extract_tool_result_payload(content)
        .and_then(|payload| payload.get(key).and_then(Value::as_array).cloned())
        .into_iter()
        .flatten()
        .filter_map(|item| item.as_str().map(str::trim).map(str::to_string))
        .filter(|value| !value.is_empty())
        .collect()
}

fn extract_json_string_field(content: &str, key: &str) -> Option<String> {
    extract_tool_result_payload(content)
        .and_then(|payload| payload.get(key).and_then(Value::as_str).map(str::to_string))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn extract_tool_result_payload(content: &str) -> Option<Value> {
    let payload = content
        .split_once("Payload:\n")
        .map(|(_, payload)| payload)
        .unwrap_or(content)
        .trim();
    serde_json::from_str(payload).ok()
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
                pending_interactions: Vec::new(),
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
    fn assemble_turn_keeps_prompt_and_runtime_views_aligned() {
        let workspace = test_workspace();
        let runtime = PromptRuntimeConfig {
            append_system_prompt: Some("appendix".to_string()),
            warnings: vec!["missing prompt file".to_string()],
            ..PromptRuntimeConfig::default()
        };
        let history = vec![Message {
            role: "user".to_string(),
            content: json!([{"type":"text","text":"hello"}]),
        }];

        let assembled = ContextAssembler::new(&workspace, &runtime).assemble_turn(
            PromptMode::Plan,
            RuntimeContextInputs {
                cwd: "repo".to_string(),
                branch: "main".to_string(),
                session_id: "session-1".to_string(),
                history_len: history.len(),
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
                pending_interactions: Vec::new(),
            },
        );

        assert!(assembled.prompt.system_prompt().contains("appendix"));
        assert_eq!(
            assembled.runtime.prompt.append_system_prompt.as_deref(),
            Some("appendix")
        );
        assert_eq!(
            assembled.runtime.prompt.warnings,
            vec!["missing prompt file".to_string()]
        );
        assert_eq!(assembled.runtime.plan.execution_mode, "plan");
        assert_eq!(assembled.runtime.session_id, "session-1");
    }

    #[test]
    fn assemble_runtime_ranks_retrieval_candidates_against_selection_budget() {
        let workspace = test_workspace();
        let runtime = PromptRuntimeConfig::default();
        let history = vec![
            Message {
                role: "assistant".to_string(),
                content: json!([
                    {
                        "type": "tool_use",
                        "id": "tool-retrieve-1",
                        "name": "retrieve_experience",
                        "input": { "query": "bootstrap contract" }
                    },
                    {
                        "type": "tool_use",
                        "id": "tool-retrieve-2",
                        "name": "retrieve_session_context",
                        "input": { "query": "previous auth flow" }
                    }
                ]),
            },
            Message {
                role: "user".to_string(),
                content: json!([
                    {
                        "type": "tool_result",
                        "tool_use_id": "tool-retrieve-1",
                        "content": "Tool retrieve_experience completed with relevant_experiences.\nPayload:\n{\n  \"relevant_experiences\": [\n    \"Prefer one shared bootstrap path.\",\n    \"Keep session restore aligned with direct execution.\"\n  ]\n}"
                    },
                    {
                        "type": "tool_result",
                        "tool_use_id": "tool-retrieve-2",
                        "content": "Tool retrieve_session_context completed with status, summary.\nPayload:\n{\n  \"status\": \"ok\",\n  \"summary\": \"Auth picker already moved behind the shared runtime bootstrap.\"\n}"
                    }
                ]),
            },
        ];

        let runtime_context = ContextAssembler::new(&workspace, &runtime).assemble_runtime(
            PromptMode::Plan,
            RuntimeContextInputs {
                cwd: "repo".to_string(),
                branch: "main".to_string(),
                session_id: "session-1".to_string(),
                history_len: history.len(),
                total_input_tokens: 11,
                total_output_tokens: 7,
                execution_mode: "plan".to_string(),
                plan_steps: Vec::new(),
                plan_explanation: None,
                compact_state: crate::agent::CompactState {
                    estimated_history_tokens: 1234,
                    context_window_tokens: Some(1_500),
                    compact_threshold_tokens: 1_420,
                    reserved_output_tokens: 1_024,
                    ..Default::default()
                },
                history: &history,
                vdb_uri: "memory://vdb",
                pending_interactions: Vec::new(),
            },
        );

        let selected_kinds = runtime_context
            .retrieval
            .memory_selection
            .selected_items
            .iter()
            .map(|item| item.kind.as_str())
            .collect::<Vec<_>>();
        let dropped_kinds = runtime_context
            .retrieval
            .memory_selection
            .dropped_items
            .iter()
            .map(|item| item.kind.as_str())
            .collect::<Vec<_>>();
        assert!(!selected_kinds.contains(&"retrieved_workspace_memory"));
        assert!(!selected_kinds.contains(&"retrieved_thread_context"));
        assert!(dropped_kinds.contains(&"retrieved_workspace_memory"));
        assert!(dropped_kinds.contains(&"retrieved_thread_context"));
        assert!(runtime_context
            .retrieval
            .memory_selection
            .dropped_items
            .iter()
            .any(|item| {
                matches!(
                    item.kind.as_str(),
                    "retrieved_workspace_memory" | "retrieved_thread_context"
                ) && item
                    .dropped_reason
                    .as_deref()
                    .is_some_and(|reason| reason.contains("memory-selection budget"))
            }));
    }

    #[test]
    fn extract_tool_result_payload_falls_back_to_plain_json_content() {
        let payload = extract_tool_result_payload(
            r#"{
                "status": "ok",
                "summary": "plain json payload without wrapper"
            }"#,
        )
        .expect("payload should parse from raw json");

        assert_eq!(payload.get("status").and_then(Value::as_str), Some("ok"));
        assert_eq!(
            payload.get("summary").and_then(Value::as_str),
            Some("plain json payload without wrapper")
        );
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
