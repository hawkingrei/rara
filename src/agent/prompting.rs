use super::*;
use crate::context::{AssembledContext, ContextAssembler};

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
