use crate::agent::PlanStepStatus;
use crate::prompt::{EffectivePrompt, PromptSource};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptSourceContextEntry {
    pub order: usize,
    pub kind: String,
    pub label: String,
    pub display_path: String,
    pub inclusion_reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptContextView {
    pub base_prompt_kind: String,
    pub section_keys: Vec<String>,
    pub source_status_lines: Vec<String>,
    pub source_entries: Vec<PromptSourceContextEntry>,
    pub warnings: Vec<String>,
}

impl PromptContextView {
    pub fn from_effective_prompt(
        effective_prompt: EffectivePrompt,
        warnings: Vec<String>,
    ) -> Self {
        Self {
            base_prompt_kind: effective_prompt.base_prompt_kind.label().to_string(),
            section_keys: effective_prompt
                .section_keys
                .iter()
                .map(|key| (*key).to_string())
                .collect(),
            source_status_lines: effective_prompt
                .sources
                .iter()
                .map(|source| source.status_line())
                .collect(),
            source_entries: prompt_source_entries(&effective_prompt.sources),
            warnings,
        }
    }
}

fn prompt_source_entries(sources: &[PromptSource]) -> Vec<PromptSourceContextEntry> {
    sources
        .iter()
        .enumerate()
        .map(|(idx, source)| PromptSourceContextEntry {
            order: idx + 1,
            kind: source.kind_label().to_string(),
            label: source.label.clone(),
            display_path: source.display_path.clone(),
            inclusion_reason: source.inclusion_reason().to_string(),
        })
        .collect()
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetrievalSelectedItemContextEntry {
    pub order: usize,
    pub kind: String,
    pub label: String,
    pub detail: String,
    pub inclusion_reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetrievalContextView {
    pub remaining_input_budget_tokens: Option<usize>,
    pub selected_items: Vec<RetrievalSelectedItemContextEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedRuntimeContext {
    pub cwd: String,
    pub branch: String,
    pub session_id: String,
    pub history_len: usize,
    pub total_input_tokens: u32,
    pub total_output_tokens: u32,
    pub prompt: PromptContextView,
    pub plan: PlanContextView,
    pub compaction: CompactionContextView,
    pub retrieval: RetrievalContextView,
}
