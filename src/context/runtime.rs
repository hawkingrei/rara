use crate::agent::{CompactState, PlanStepStatus};
use crate::prompt::{EffectivePrompt, PromptSource};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptSourceContextEntry {
    pub order: usize,
    pub kind: String,
    pub label: String,
    pub display_path: String,
    pub status_line: String,
    pub inclusion_reason: String,
}

impl PromptSourceContextEntry {
    fn from_prompt_source(order: usize, source: &PromptSource) -> Self {
        Self {
            order,
            kind: source.kind_label().to_string(),
            label: source.label.clone(),
            display_path: source.display_path.clone(),
            status_line: source.status_line(),
            inclusion_reason: source.inclusion_reason().to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptContextView {
    pub base_prompt_kind: String,
    pub section_keys: Vec<String>,
    pub source_entries: Vec<PromptSourceContextEntry>,
    pub source_status_lines: Vec<String>,
    pub append_system_prompt: Option<String>,
    pub warnings: Vec<String>,
}

impl PromptContextView {
    pub fn from_effective_prompt(
        effective_prompt: EffectivePrompt,
        append_system_prompt: Option<String>,
        warnings: Vec<String>,
    ) -> Self {
        Self {
            base_prompt_kind: effective_prompt.base_prompt_kind.label().to_string(),
            section_keys: effective_prompt
                .section_keys
                .iter()
                .map(|key| (*key).to_string())
                .collect(),
            source_entries: effective_prompt
                .sources
                .iter()
                .enumerate()
                .map(|(idx, source)| PromptSourceContextEntry::from_prompt_source(idx + 1, source))
                .collect(),
            source_status_lines: effective_prompt
                .sources
                .iter()
                .map(|source| source.status_line())
                .collect(),
            append_system_prompt,
            warnings,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanContextView {
    pub execution_mode: String,
    pub steps: Vec<(String, String)>,
    pub explanation: Option<String>,
}

impl PlanContextView {
    pub fn from_agent_state(
        execution_mode: &str,
        steps: impl Iterator<Item = (PlanStepStatus, String)>,
        explanation: Option<String>,
    ) -> Self {
        Self {
            execution_mode: execution_mode.to_string(),
            steps: steps
                .map(|(status, step)| {
                    let status = match status {
                        PlanStepStatus::Pending => "pending",
                        PlanStepStatus::InProgress => "in_progress",
                        PlanStepStatus::Completed => "completed",
                    };
                    (status.to_string(), step)
                })
                .collect(),
            explanation,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextBudgetView {
    pub context_window_tokens: Option<usize>,
    pub compact_threshold_tokens: usize,
    pub reserved_output_tokens: usize,
}

impl ContextBudgetView {
    pub fn from_compact_state(state: &CompactState) -> Self {
        Self {
            context_window_tokens: state.context_window_tokens,
            compact_threshold_tokens: state.compact_threshold_tokens,
            reserved_output_tokens: state.reserved_output_tokens,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionContextView {
    pub estimated_history_tokens: usize,
    pub context_window_tokens: Option<usize>,
    pub compact_threshold_tokens: usize,
    pub reserved_output_tokens: usize,
    pub compaction_count: usize,
    pub last_compaction_before_tokens: Option<usize>,
    pub last_compaction_after_tokens: Option<usize>,
    pub last_compaction_recent_files: Vec<String>,
    pub last_compaction_boundary_version: Option<u32>,
    pub last_compaction_boundary_before_tokens: Option<usize>,
    pub last_compaction_boundary_recent_file_count: Option<usize>,
    pub source_entries: Vec<CompactionSourceContextEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionSourceContextEntry {
    pub order: usize,
    pub kind: String,
    pub label: String,
    pub detail: String,
    pub inclusion_reason: String,
}

impl CompactionContextView {
    pub fn from_compact_state(state: &CompactState) -> Self {
        Self {
            estimated_history_tokens: state.estimated_history_tokens,
            context_window_tokens: state.context_window_tokens,
            compact_threshold_tokens: state.compact_threshold_tokens,
            reserved_output_tokens: state.reserved_output_tokens,
            compaction_count: state.compaction_count,
            last_compaction_before_tokens: state.last_compaction_before_tokens,
            last_compaction_after_tokens: state.last_compaction_after_tokens,
            last_compaction_recent_files: state.last_compaction_recent_files.clone(),
            last_compaction_boundary_version: state
                .last_compaction_boundary
                .map(|boundary| boundary.version),
            last_compaction_boundary_before_tokens: state
                .last_compaction_boundary
                .map(|boundary| boundary.before_tokens),
            last_compaction_boundary_recent_file_count: state
                .last_compaction_boundary
                .map(|boundary| boundary.recent_file_count),
            source_entries: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedRuntimeContext {
    pub cwd: String,
    pub branch: String,
    pub session_id: String,
    pub history_len: usize,
    pub total_input_tokens: u32,
    pub total_output_tokens: u32,
    pub budget: ContextBudgetView,
    pub prompt: PromptContextView,
    pub plan: PlanContextView,
    pub compaction: CompactionContextView,
    pub retrieval: RetrievalContextView,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetrievalContextView {
    pub entries: Vec<RetrievalSourceContextEntry>,
    pub selected_items: Vec<RetrievalSelectedItemContextEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetrievalSourceContextEntry {
    pub order: usize,
    pub kind: String,
    pub label: String,
    pub status: String,
    pub detail: String,
    pub inclusion_reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetrievalSelectedItemContextEntry {
    pub order: usize,
    pub kind: String,
    pub label: String,
    pub detail: String,
    pub inclusion_reason: String,
}
