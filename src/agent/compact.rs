use super::*;
use crate::llm::ContextBudget;

#[derive(Debug, Clone, Default)]
pub struct CompactState {
    pub estimated_history_tokens: usize,
    pub context_window_tokens: Option<usize>,
    pub compact_threshold_tokens: usize,
    pub reserved_output_tokens: usize,
    pub compaction_count: usize,
    pub last_compaction_before_tokens: Option<usize>,
    pub last_compaction_after_tokens: Option<usize>,
}

impl Agent {
    pub async fn compact_if_needed(&mut self) -> Result<()> {
        self.compact_if_needed_with_reporter(|_| {}).await
    }

    pub async fn compact_if_needed_with_reporter<F>(&mut self, mut report: F) -> Result<()>
    where
        F: FnMut(AgentEvent) + Send,
    {
        self.compact_history_with_reporter(false, &mut report).await
    }

    pub async fn compact_now_with_reporter<F>(&mut self, mut report: F) -> Result<bool>
    where
        F: FnMut(AgentEvent) + Send,
    {
        self.compact_history_with_reporter(true, &mut report).await?;
        Ok(self.compact_state.last_compaction_before_tokens.is_some())
    }

    async fn compact_history_with_reporter<F>(&mut self, force: bool, report: &mut F) -> Result<()>
    where
        F: FnMut(AgentEvent) + Send,
    {
        let current_tokens = estimate_history_tokens(&self.history)?;
        let compact_budget = self.current_compact_budget();
        self.compact_state.estimated_history_tokens = current_tokens;
        self.compact_state.context_window_tokens =
            compact_budget.as_ref().map(|budget| budget.context_window_tokens);
        self.compact_state.compact_threshold_tokens =
            compact_budget.as_ref().map(|budget| budget.compact_threshold_tokens).unwrap_or(10_000);
        self.compact_state.reserved_output_tokens =
            compact_budget.as_ref().map(|budget| budget.reserved_output_tokens).unwrap_or(0);
        self.compact_state.last_compaction_before_tokens = None;
        self.compact_state.last_compaction_after_tokens = None;

        let threshold = self.compact_state.compact_threshold_tokens;
        if !force && current_tokens <= threshold {
            return Ok(());
        }
        if self.history.len() < 2 {
            return Ok(());
        }

        report(AgentEvent::Status(if force {
            "Compacting conversation history on demand.".to_string()
        } else {
            "Compacting long conversation history.".to_string()
        }));

        let split_idx = (self.history.len() as f64 * 0.8) as usize;
        let split_idx = split_idx.clamp(1, self.history.len().saturating_sub(1));
        let summary = self.llm_backend.summarize(&self.history[..split_idx]).await?;
        let mut new_history = vec![Message {
            role: "system".to_string(),
            content: json!(format!("SUMMARY OF PREVIOUS CONVERSATION: {}", summary)),
        }];
        new_history.extend_from_slice(&self.history[split_idx..]);
        self.history = new_history;
        self.session_manager.save_session(&self.session_id, &self.history)?;

        let compacted_tokens = estimate_history_tokens(&self.history)?;
        self.compact_state.estimated_history_tokens = compacted_tokens;
        self.compact_state.compaction_count += 1;
        self.compact_state.last_compaction_before_tokens = Some(current_tokens);
        self.compact_state.last_compaction_after_tokens = Some(compacted_tokens);
        Ok(())
    }

    pub(super) fn current_compact_budget(&self) -> Option<ContextBudget> {
        let tools = self.visible_tool_schemas();
        self.llm_backend.context_budget(&self.history, &tools)
    }
}

fn estimate_history_tokens(history: &[Message]) -> Result<usize> {
    let bpe = tiktoken_rs::cl100k_base()?;
    Ok(history
        .iter()
        .map(|message| bpe.encode_with_special_tokens(&message.content.to_string()).len())
        .sum())
}
