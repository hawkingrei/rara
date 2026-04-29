mod compaction;
mod memory_selection;
mod view;

use crate::agent::{CompactState, Message, PlanStepStatus};
use crate::context::{
    CompactionContextView, ContextBudgetView, PlanContextView, PromptContextView,
    RetrievalContextView, SharedRuntimeContext,
};
use crate::llm::{ContextBudget, LlmBackend};
use crate::prompt::{self, EffectivePrompt, PromptMode, PromptRuntimeConfig};
use crate::workspace::WorkspaceMemory;
use serde_json::Value;

use view::{
    active_turn_budget, assemble_context_view, compaction_context_entries, estimate_text_tokens,
    retrieval_context,
};

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
        let system_prompt_budget = estimate_text_tokens(effective_prompt.text.as_str());
        let stable_instructions_budget = system_prompt_budget;
        let workspace_prompt_budget = effective_prompt
            .sources
            .iter()
            .filter(|source| matches!(source.kind_label(), "project_instruction" | "local_memory"))
            .map(|source| estimate_text_tokens(source.content.as_str()))
            .sum();
        let compacted_history_budget = effective_prompt
            .sources
            .iter()
            .filter(|source| source.kind_label() == "compacted_history")
            .map(|source| estimate_text_tokens(source.content.as_str()))
            .sum::<usize>();
        let active_turn_budget_value = active_turn_budget(
            inputs.plan_explanation.as_deref(),
            inputs.plan_steps.as_slice(),
            inputs.pending_interactions.as_slice(),
            inputs.history,
        );
        let retrieved_memory_budget = 0_usize;
        let plan_explanation = inputs.plan_explanation.as_deref();
        let plan_steps = inputs.plan_steps.as_slice();
        let pending_interactions = inputs.pending_interactions.as_slice();
        let compact_entries = compaction_context_entries(inputs.history);

        let retrieval = retrieval_context(
            effective_prompt.sources.as_slice(),
            plan_explanation,
            plan_steps,
            pending_interactions,
            inputs.history,
            inputs.session_id.as_str(),
            inputs.vdb_uri,
            &inputs.compact_state,
            self.workspace,
        );

        let assembly = assemble_context_view(
            &effective_prompt,
            plan_explanation,
            plan_steps,
            pending_interactions,
            compact_entries.as_slice(),
            retrieval.memory_selection.selected_items.as_slice(),
            retrieval.memory_selection.available_items.as_slice(),
            retrieval.memory_selection.dropped_items.as_slice(),
            inputs.history,
        );

        let budget = ContextBudgetView::from_compact_state(
            &inputs.compact_state,
            system_prompt_budget,
            stable_instructions_budget,
            workspace_prompt_budget,
            active_turn_budget_value,
            compacted_history_budget,
            retrieved_memory_budget,
        );

        let mut compaction = CompactionContextView::from_compact_state(&inputs.compact_state);
        compaction.source_entries = compact_entries;

        SharedRuntimeContext {
            cwd: inputs.cwd,
            branch: inputs.branch,
            session_id: inputs.session_id,
            history_len: inputs.history_len,
            total_input_tokens: inputs.total_input_tokens,
            total_output_tokens: inputs.total_output_tokens,
            budget,
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
        assert!(
            assembled
                .effective_prompt
                .section_keys
                .contains(&"append_system_prompt")
        );
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

        assert_eq!(
            runtime_context.prompt.append_system_prompt.as_deref(),
            Some("appendix")
        );
        assert_eq!(
            runtime_context.prompt.warnings,
            vec!["missing prompt file".to_string()]
        );
        assert_eq!(runtime_context.plan.execution_mode, "plan");
        assert_eq!(runtime_context.session_id, "session-1");
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
        assert!(
            runtime_context
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
                })
        );
    }

    #[test]
    fn assemble_runtime_includes_active_thread_working_set_in_memory_selection() {
        let workspace = test_workspace();
        let runtime = PromptRuntimeConfig::default();
        let history = vec![
            Message {
                role: "user".to_string(),
                content: json!([{"type":"text","text":"please continue the bootstrap cleanup"}]),
            },
            Message {
                role: "user".to_string(),
                content: json!([
                    {
                        "type": "tool_result",
                        "tool_use_id": "tool-shell-1",
                        "content": "diff preview"
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
                plan_steps: vec![(PlanStepStatus::Pending, "inspect bootstrap".to_string())],
                plan_explanation: Some("Keep one assembly path.".to_string()),
                compact_state: crate::agent::CompactState {
                    context_window_tokens: Some(8_192),
                    compact_threshold_tokens: 7_000,
                    reserved_output_tokens: 1_024,
                    ..Default::default()
                },
                history: &history,
                vdb_uri: "",
                pending_interactions: vec![RuntimeInteractionInput {
                    kind: "approval".to_string(),
                    title: "Approve shell command".to_string(),
                    summary: "Allow one shell command in the repo root.".to_string(),
                    source: None,
                }],
            },
        );

        let selected_kinds = runtime_context
            .retrieval
            .memory_selection
            .selected_items
            .iter()
            .map(|item| item.kind.as_str())
            .collect::<Vec<_>>();
        assert!(selected_kinds.contains(&"plan_explanation"));
        assert!(selected_kinds.contains(&"plan_steps"));
        assert!(selected_kinds.contains(&"approval"));
        assert!(selected_kinds.contains(&"latest_user_request"));
        assert!(selected_kinds.contains(&"tool_result"));
    }

    fn extract_tool_result_payload(content: &str) -> Option<Value> {
        let payload = content
            .split_once("Payload:\n")
            .map(|(_, payload)| payload)
            .unwrap_or(content)
            .trim();
        serde_json::from_str(payload).ok()
    }

    #[test]
    fn extract_tool_result_payload_falls_back_to_plain_json_content() {
        let payload = extract_tool_result_payload(
            r#"{
                "status": "ok",
                "summary": "plain json payload without wrapper"
            }"#,
        )
        .expect("payload should parse");
        assert_eq!(payload["status"].as_str(), Some("ok"));
        assert_eq!(
            payload["summary"].as_str(),
            Some("plain json payload without wrapper")
        );
    }
}
