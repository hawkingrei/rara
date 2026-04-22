use crate::agent::Message;
use crate::llm::{ContextBudget, LlmBackend};
use crate::prompt::{self, EffectivePrompt, PromptMode, PromptRuntimeConfig};
use crate::workspace::WorkspaceMemory;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssembledContext {
    pub effective_prompt: EffectivePrompt,
    pub compact_instruction: String,
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
    use crate::agent::{AnthropicResponse, ContentBlock};
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
        async fn ask(&self, _messages: &[Message], _tools: &[Value]) -> Result<AnthropicResponse> {
            Ok(AnthropicResponse {
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
