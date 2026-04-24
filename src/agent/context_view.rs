use super::*;
use crate::context::{
    CompactionContextView, PlanContextView, PromptContextView, SharedRuntimeContext,
};

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
        }
    }
}
