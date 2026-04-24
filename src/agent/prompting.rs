use super::*;
use crate::context::{AssembledContext, ContextAssembler, RuntimeContextInputs};

impl Agent {
    pub fn assemble_context(&self) -> AssembledContext {
        self.context_assembler().assemble(match self.execution_mode {
            AgentExecutionMode::Execute => PromptMode::Execute,
            AgentExecutionMode::Plan => PromptMode::Plan,
        })
    }

    pub(super) fn context_assembler(&self) -> ContextAssembler<'_> {
        ContextAssembler::new(&self.workspace, &self.prompt_config)
    }

    pub fn assemble_runtime_context(&self) -> crate::context::SharedRuntimeContext {
        let (cwd, branch) = self.workspace.get_env_info();
        let mode = match self.execution_mode {
            AgentExecutionMode::Execute => PromptMode::Execute,
            AgentExecutionMode::Plan => PromptMode::Plan,
        };
        self.context_assembler().assemble_runtime(
            mode,
            RuntimeContextInputs {
                cwd,
                branch,
                session_id: self.session_id.clone(),
                history_len: self.history.len(),
                total_input_tokens: self.total_input_tokens,
                total_output_tokens: self.total_output_tokens,
                execution_mode: self.execution_mode_label().to_string(),
                plan_steps: self
                    .current_plan
                    .iter()
                    .map(|step| (step.status.clone(), step.step.clone()))
                    .collect(),
                plan_explanation: self.plan_explanation.clone(),
                compact_state: self.compact_state.clone(),
                history: &self.history,
                vdb_uri: self.vdb.uri(),
            },
        )
    }

    pub fn build_system_prompt(&self) -> String {
        self.assemble_context().effective_prompt.text
    }

    pub fn effective_prompt(&self) -> prompt::EffectivePrompt {
        self.assemble_context().effective_prompt
    }

    pub fn set_prompt_config(&mut self, prompt_config: PromptRuntimeConfig) {
        self.prompt_config = prompt_config;
    }

    pub fn prompt_config(&self) -> &PromptRuntimeConfig {
        &self.prompt_config
    }
}
