use super::*;

impl Agent {
    pub fn build_system_prompt(&self) -> String {
        prompt::build_system_prompt(
            &self.workspace,
            &self.prompt_config,
            match self.execution_mode {
                AgentExecutionMode::Execute => PromptMode::Execute,
                AgentExecutionMode::Plan => PromptMode::Plan,
            },
        )
    }

    pub fn effective_prompt(&self) -> prompt::EffectivePrompt {
        prompt::build_effective_prompt(
            &self.workspace,
            &self.prompt_config,
            match self.execution_mode {
                AgentExecutionMode::Execute => PromptMode::Execute,
                AgentExecutionMode::Plan => PromptMode::Plan,
            },
        )
    }

    pub fn set_prompt_config(&mut self, prompt_config: PromptRuntimeConfig) {
        self.prompt_config = prompt_config;
    }

    pub fn prompt_config(&self) -> &PromptRuntimeConfig {
        &self.prompt_config
    }
}
