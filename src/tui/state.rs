mod state_presets;

use ratatui::text::Line;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::JoinHandle;

pub use self::state_presets::{
    current_model_presets, selected_preset_idx_for_config, selected_provider_family_idx_for_config,
};
use super::markdown_stream::MarkdownStreamCollector;
use crate::agent::{Agent, AgentExecutionMode, BashApprovalMode};
use crate::config::{ConfigManager, RaraConfig};
use crate::redaction::redact_secrets;
use crate::state_db::{
    PersistedInteraction, PersistedPlanStep, PersistedSessionSummary, PersistedTurnEntry, StateDb,
};
use crate::tools::bash::BashCommandInput;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HelpTab {
    General,
    Commands,
    Runtime,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Overlay {
    Help(HelpTab),
    CommandPalette,
    Status,
    Setup,
    ProviderPicker,
    ModelPicker,
    ResumePicker,
    BaseUrlEditor,
    CodexAuthGuide,
    ApiKeyEditor,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProviderFamily {
    Codex,
    CandleLocal,
    Ollama,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LocalCommandKind {
    Help,
    Status,
    Clear,
    Resume,
    Plan,
    Approval,
    Compact,
    Setup,
    Model,
    BaseUrl,
    Login,
    Quit,
}

pub struct LocalCommand {
    pub kind: LocalCommandKind,
    pub arg: Option<String>,
}

pub struct CommandSpec {
    pub category: &'static str,
    pub name: &'static str,
    pub usage: &'static str,
    pub summary: &'static str,
    pub detail: &'static str,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RuntimePhase {
    Idle,
    LocalCommand,
    SendingPrompt,
    ProcessingResponse,
    RunningTool,
    RebuildingBackend,
    BackendReady,
    OAuthStarting,
    OAuthWaitingCallback,
    OAuthExchangingToken,
    OAuthSaved,
    Failed,
}

impl Default for RuntimePhase {
    fn default() -> Self {
        Self::Idle
    }
}

#[derive(Default, Clone)]
pub struct RuntimeSnapshot {
    pub cwd: String,
    pub branch: String,
    pub session_id: String,
    pub history_len: usize,
    pub total_input_tokens: u32,
    pub total_output_tokens: u32,
    pub estimated_history_tokens: usize,
    pub context_window_tokens: Option<usize>,
    pub compact_threshold_tokens: usize,
    pub reserved_output_tokens: usize,
    pub compaction_count: usize,
    pub last_compaction_before_tokens: Option<usize>,
    pub last_compaction_after_tokens: Option<usize>,
    pub plan_steps: Vec<(String, String)>,
    pub plan_explanation: Option<String>,
    pub pending_interactions: Vec<PendingInteractionSnapshot>,
    pub completed_interactions: Vec<CompletedInteractionSnapshot>,
    pub prompt_base_kind: String,
    pub prompt_section_keys: Vec<String>,
    pub prompt_source_status_lines: Vec<String>,
    pub prompt_warnings: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InteractionKind {
    RequestInput,
    Approval,
    PlanApproval,
}

#[derive(Default, Clone)]
pub struct PendingApprovalSnapshot {
    pub tool_use_id: String,
    pub command: String,
    pub allow_net: bool,
    pub payload: BashCommandInput,
}

#[derive(Clone)]
pub struct PendingInteractionSnapshot {
    pub kind: InteractionKind,
    pub title: String,
    pub summary: String,
    pub options: Vec<(String, String)>,
    pub note: Option<String>,
    pub approval: Option<PendingApprovalSnapshot>,
    pub source: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivePendingInteractionKind {
    PlanApproval,
    ShellApproval,
    PlanningQuestion,
    ExplorationQuestion,
    SubAgentQuestion,
    RequestInput,
}

pub struct ActivePendingInteraction<'a> {
    pub kind: ActivePendingInteractionKind,
    pub _snapshot: &'a PendingInteractionSnapshot,
}

#[derive(Clone)]
pub struct CompletedInteractionSnapshot {
    pub kind: InteractionKind,
    pub title: String,
    pub summary: String,
    pub source: Option<String>,
}

fn completed_interaction_role(kind: InteractionKind, source: Option<&str>) -> &'static str {
    match kind {
        InteractionKind::Approval => "Shell Approval Completed",
        InteractionKind::PlanApproval => "Plan Decision",
        InteractionKind::RequestInput => match source {
            Some("plan_agent") => "Planning Question Answered",
            Some("explore_agent") => "Exploration Question Answered",
            Some(_) => "Sub-agent Question Answered",
            None => "Question Answered",
        },
    }
}

pub enum TaskKind {
    Query,
    Compact,
    Rebuild,
    OAuth,
}

pub enum TaskCompletion {
    Query {
        agent: Agent,
        result: anyhow::Result<()>,
    },
    Compact {
        agent: Agent,
        result: anyhow::Result<bool>,
    },
    Rebuild {
        result: anyhow::Result<Agent>,
    },
    OAuth {
        result: anyhow::Result<secrecy::SecretString>,
    },
}

pub enum TuiEvent {
    Transcript { role: &'static str, message: String },
}

pub struct RunningTask {
    pub kind: TaskKind,
    pub receiver: UnboundedReceiver<TuiEvent>,
    pub handle: JoinHandle<TaskCompletion>,
    pub started_at: Instant,
    pub next_heartbeat_after_secs: u64,
}

pub const PROVIDER_FAMILIES: [(ProviderFamily, &str, &str); 3] = [
    (
        ProviderFamily::Codex,
        "Codex",
        "Use the Codex-compatible API with either OAuth login or an API key.",
    ),
    (
        ProviderFamily::CandleLocal,
        "Candle Local",
        "Run local Candle models directly in-process.",
    ),
    (
        ProviderFamily::Ollama,
        "Ollama",
        "Use an external Ollama server and choose a local tag.",
    ),
];

#[derive(Clone, Default)]
pub struct TranscriptEntry {
    pub role: String,
    pub message: String,
}

#[derive(Clone, Default)]
pub struct TranscriptTurn {
    pub entries: Vec<TranscriptEntry>,
}

pub struct AgentMarkdownStreamState {
    raw_text: String,
    collector: MarkdownStreamCollector,
    committed_lines: Vec<Line<'static>>,
    display_lines: Vec<Line<'static>>,
}

impl AgentMarkdownStreamState {
    fn new(cwd: PathBuf) -> Self {
        Self {
            raw_text: String::new(),
            collector: MarkdownStreamCollector::new(None, &cwd),
            committed_lines: Vec::new(),
            display_lines: Vec::new(),
        }
    }

    fn push_delta(&mut self, delta: &str) {
        self.raw_text.push_str(delta);
        self.collector.push_delta(delta);
        self.committed_lines
            .extend(self.collector.commit_complete_lines());
        self.display_lines = self.committed_lines.clone();
        self.display_lines.extend(self.collector.preview_lines());
    }
}

#[derive(Default)]
pub struct ActiveLiveSections {
    pub exploration_actions: Vec<String>,
    pub exploration_notes: Vec<String>,
    pub planning_actions: Vec<String>,
    pub planning_notes: Vec<String>,
    pub running_actions: Vec<String>,
}

pub struct TuiApp {
    pub input: String,
    pub committed_turns: Vec<TranscriptTurn>,
    pub active_turn: TranscriptTurn,
    pub startup_card_inserted: bool,
    pub inserted_turns: usize,
    pub overlay: Option<Overlay>,
    pub config: RaraConfig,
    pub config_manager: ConfigManager,
    pub setup_status: Option<String>,
    pub notice: Option<String>,
    pub runtime_phase: RuntimePhase,
    pub runtime_phase_detail: Option<String>,
    pub snapshot: RuntimeSnapshot,
    pub agent_execution_mode: AgentExecutionMode,
    pub bash_approval_mode: BashApprovalMode,
    pub provider_picker_idx: usize,
    pub model_picker_idx: usize,
    pub command_palette_idx: usize,
    pub base_url_input: String,
    pub api_key_input: String,
    pub recent_commands: Vec<String>,
    pub recent_sessions: Vec<PersistedSessionSummary>,
    pub resume_picker_idx: usize,
    pub transcript_scroll: usize,
    pub agent_markdown_stream: Option<AgentMarkdownStreamState>,
    pub active_live: ActiveLiveSections,
    pub pending_planning_suggestion: Option<String>,
    pub terminal_focused: bool,
    pub state_db: Option<Arc<StateDb>>,
    pub state_db_status: Option<String>,
    pub running_task: Option<RunningTask>,
}

pub fn input_requests_command_palette(input: &str) -> bool {
    input.trim_start().starts_with('/')
}

pub(crate) fn contains_structured_planning_output(message: &str) -> bool {
    message.contains("<plan>") || message.contains("<request_user_input>")
}

fn state_db_status_error(prefix: &str, message: impl Into<String>) -> String {
    format!("{prefix}: {}", redact_secrets(message.into()))
}

impl TuiApp {
    fn ensure_completed_interaction_entry(
        &mut self,
        kind: InteractionKind,
        title: &str,
        summary: &str,
        source: Option<&str>,
    ) {
        let role = completed_interaction_role(kind, source).to_string();
        let message = format!("{title}: {summary}");
        let exists = self
            .active_turn
            .entries
            .iter()
            .chain(
                self.committed_turns
                    .iter()
                    .flat_map(|turn| turn.entries.iter()),
            )
            .any(|entry| entry.role == role && entry.message == message);
        if !exists {
            self.active_turn.entries.push(TranscriptEntry { role, message });
        }
    }

    fn plan_approval_interaction(&self) -> PendingInteractionSnapshot {
        PendingInteractionSnapshot {
            kind: InteractionKind::PlanApproval,
            title: "Plan Ready".to_string(),
            summary: self
                .snapshot
                .plan_explanation
                .clone()
                .unwrap_or_else(|| "Review the proposed plan before implementation.".to_string()),
            options: Vec::new(),
            note: None,
            approval: None,
            source: None,
        }
    }

    fn set_plan_approval_interaction(&mut self, pending: bool) {
        self.snapshot
            .pending_interactions
            .retain(|item| item.kind != InteractionKind::PlanApproval);
        if pending {
            self.snapshot
                .pending_interactions
                .push(self.plan_approval_interaction());
        }
    }

    pub fn new(cm: ConfigManager) -> anyhow::Result<Self> {
        let cfg = cm.load()?;
        let overlay = if !cfg.has_api_key() && super::provider_requires_api_key(&cfg.provider) {
            if cfg.provider == "codex" {
                Some(Overlay::CodexAuthGuide)
            } else {
                Some(Overlay::Setup)
            }
        } else {
            None
        };
        let provider_picker_idx = selected_provider_family_idx_for_config(&cfg);
        let model_picker_idx = selected_preset_idx_for_config(&cfg, provider_picker_idx);
        Ok(Self {
            input: String::new(),
            committed_turns: Vec::new(),
            active_turn: TranscriptTurn::default(),
            startup_card_inserted: false,
            inserted_turns: 0,
            overlay,
            config: cfg,
            config_manager: cm,
            setup_status: None,
            notice: None,
            runtime_phase: RuntimePhase::Idle,
            runtime_phase_detail: None,
            snapshot: RuntimeSnapshot::default(),
            agent_execution_mode: AgentExecutionMode::Execute,
            bash_approval_mode: BashApprovalMode::Suggestion,
            provider_picker_idx,
            model_picker_idx,
            command_palette_idx: 0,
            base_url_input: String::new(),
            api_key_input: String::new(),
            recent_commands: Vec::new(),
            recent_sessions: Vec::new(),
            resume_picker_idx: 0,
            transcript_scroll: 0,
            agent_markdown_stream: None,
            active_live: ActiveLiveSections::default(),
            pending_planning_suggestion: None,
            terminal_focused: true,
            state_db: None,
            state_db_status: None,
            running_task: None,
        })
    }

    pub fn is_busy(&self) -> bool {
        self.running_task.is_some()
    }

    pub fn current_model_label(&self) -> &str {
        self.config.model.as_deref().unwrap_or("-")
    }

    pub fn selected_preset_idx(&self) -> usize {
        selected_preset_idx_for_config(&self.config, self.provider_picker_idx)
    }

    pub fn selected_provider_family(&self) -> ProviderFamily {
        PROVIDER_FAMILIES[self.provider_picker_idx].0
    }

    pub fn select_local_model(&mut self, idx: usize) {
        let presets = current_model_presets(self.provider_picker_idx);
        let (_, provider, model) = presets[idx];
        self.model_picker_idx = idx;
        self.config.provider = provider.to_string();
        self.config.model = Some(model.to_string());
        if provider == "ollama" {
            self.config.revision = None;
            if self
                .config
                .base_url
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_none()
            {
                self.config.base_url = Some("http://localhost:11434".to_string());
            }
        } else if provider == "codex" {
            self.config.revision = None;
            if self
                .config
                .base_url
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_none()
            {
                self.config.base_url = Some("http://localhost:8080".to_string());
            }
        } else {
            self.config.revision = Some("main".to_string());
            self.config.base_url = None;
        }
    }

    pub fn cycle_local_model(&mut self) {
        let next = (self.selected_preset_idx() + 1)
            % current_model_presets(self.provider_picker_idx).len();
        self.select_local_model(next);
    }

    pub fn sync_snapshot(&mut self, agent: &Agent) {
        let (cwd, branch) = agent.workspace.get_env_info();
        let effective_prompt = agent.effective_prompt();
        let existing_plan_completion = self
            .completed_interaction(InteractionKind::PlanApproval)
            .cloned();
        let existing_pending_plan_approval =
            self.pending_plan_approval_interaction().cloned();
        let existing_local_request_completion = self
            .snapshot
            .completed_interactions
            .iter()
            .find(|item| {
                item.kind == InteractionKind::RequestInput
                    && item.source.as_deref().is_some()
            })
            .cloned();
        let existing_local_request_inputs = self
            .snapshot
            .pending_interactions
            .iter()
            .filter(|item| {
                item.kind == InteractionKind::RequestInput
                    && item.source.as_deref().is_some()
            })
            .cloned()
            .collect::<Vec<_>>();
        let mut pending_interactions = Vec::new();
        if let Some(question) = agent.pending_user_input.as_ref() {
            pending_interactions.push(PendingInteractionSnapshot {
                kind: InteractionKind::RequestInput,
                title: question.question.clone(),
                summary: question.note.clone().unwrap_or_default(),
                options: question.options.clone(),
                note: question.note.clone(),
                approval: None,
                source: None,
            });
        }
        if agent.pending_user_input.is_none() {
            pending_interactions.extend(existing_local_request_inputs);
        }
        if let Some(item) = existing_pending_plan_approval {
            pending_interactions.push(item);
        }
        if let Some(pending) = agent.pending_approval.as_ref() {
            pending_interactions.push(PendingInteractionSnapshot {
                kind: InteractionKind::Approval,
                title: "Pending Approval".to_string(),
                summary: pending.request.summary(),
                options: Vec::new(),
                note: None,
                approval: Some(PendingApprovalSnapshot {
                    tool_use_id: pending.tool_use_id.clone(),
                    command: pending.request.summary(),
                    allow_net: pending.request.allow_net,
                    payload: pending.request.clone(),
                }),
                source: None,
            });
        }
        let mut completed_interactions = Vec::new();
        if let Some(item) = agent.completed_user_input.as_ref() {
            completed_interactions.push(CompletedInteractionSnapshot {
                kind: InteractionKind::RequestInput,
                title: item.title.clone(),
                summary: item.summary.clone(),
                source: None,
            });
        }
        if let Some(item) = agent.completed_approval.as_ref() {
            completed_interactions.push(CompletedInteractionSnapshot {
                kind: InteractionKind::Approval,
                title: item.title.clone(),
                summary: item.summary.clone(),
                source: None,
            });
        }
        if let Some(item) = existing_local_request_completion {
            completed_interactions.push(item);
        }
        if let Some(item) = existing_plan_completion {
            completed_interactions.push(item);
        }
        for interaction in &completed_interactions {
            self.ensure_completed_interaction_entry(
                interaction.kind,
                interaction.title.as_str(),
                interaction.summary.as_str(),
                interaction.source.as_deref(),
            );
        }
        self.snapshot = RuntimeSnapshot {
            cwd,
            branch,
            session_id: agent.session_id.clone(),
            history_len: agent.history.len(),
            total_input_tokens: agent.total_input_tokens,
            total_output_tokens: agent.total_output_tokens,
            estimated_history_tokens: agent.compact_state.estimated_history_tokens,
            context_window_tokens: agent.compact_state.context_window_tokens,
            compact_threshold_tokens: agent.compact_state.compact_threshold_tokens,
            reserved_output_tokens: agent.compact_state.reserved_output_tokens,
            compaction_count: agent.compact_state.compaction_count,
            last_compaction_before_tokens: agent.compact_state.last_compaction_before_tokens,
            last_compaction_after_tokens: agent.compact_state.last_compaction_after_tokens,
            plan_steps: agent
                .current_plan
                .iter()
                .map(|step| {
                    let status = match step.status {
                        crate::agent::PlanStepStatus::Pending => "pending",
                        crate::agent::PlanStepStatus::InProgress => "in_progress",
                        crate::agent::PlanStepStatus::Completed => "completed",
                    };
                    (status.to_string(), step.step.clone())
            })
                .collect(),
            plan_explanation: agent.plan_explanation.clone(),
            pending_interactions,
            completed_interactions,
            prompt_base_kind: effective_prompt.base_prompt_kind.label().to_string(),
            prompt_section_keys: effective_prompt
                .section_keys
                .iter()
                .map(|key| (*key).to_string())
                .collect(),
            prompt_source_status_lines: effective_prompt
                .sources
                .iter()
                .map(|source| source.status_line())
                .collect(),
            prompt_warnings: agent.prompt_config().warnings.clone(),
        };
        self.agent_execution_mode = agent.execution_mode;
        self.bash_approval_mode = agent.bash_approval_mode;
        self.persist_runtime_state();
    }

    pub fn push_entry(&mut self, role: &'static str, message: impl Into<String>) {
        let message = match role {
            "System" | "Runtime" => redact_secrets(message.into()),
            _ => message.into(),
        };
        if role == "You" && !self.active_turn.entries.is_empty() {
            self.commit_active_turn();
        }
        self.active_turn.entries.push(TranscriptEntry {
            role: role.to_string(),
            message,
        });
        self.transcript_scroll = 0;
    }

    pub fn append_to_latest_entry(&mut self, role: &'static str, delta: &str) {
        if let Some(last) = self.active_turn.entries.last_mut() {
            if last.role == role {
                last.message.push_str(delta);
                self.transcript_scroll = 0;
                return;
            }
        }
        self.push_entry(role, delta.to_string());
    }

    pub fn append_agent_delta(&mut self, delta: &str) {
        let cwd = if !self.snapshot.cwd.is_empty() {
            PathBuf::from(self.snapshot.cwd.as_str())
        } else {
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
        };
        let stream = self
            .agent_markdown_stream
            .get_or_insert_with(|| AgentMarkdownStreamState::new(cwd));
        stream.push_delta(delta);
        self.transcript_scroll = 0;
    }

    pub fn agent_stream_lines(&self) -> Option<&[Line<'static>]> {
        self.agent_markdown_stream
            .as_ref()
            .map(|stream| stream.display_lines.as_slice())
    }

    pub fn finalize_agent_stream(&mut self, final_message: Option<String>) {
        let fallback = self
            .agent_markdown_stream
            .take()
            .map(|stream| stream.raw_text)
            .filter(|text| !text.is_empty());
        let Some(message) = final_message.or(fallback) else {
            return;
        };

        if let Some(last) = self.active_turn.entries.last_mut() {
            if last.role == "Agent" {
                last.message = message;
                self.transcript_scroll = 0;
                return;
            }
        }
        self.push_entry("Agent", message);
    }

    pub fn push_notice(&mut self, message: impl Into<String>) {
        let message = redact_secrets(message.into());
        self.notice = Some(message.clone());
        self.push_entry("System", message);
    }

    pub fn reset_transcript(&mut self) {
        self.committed_turns.clear();
        self.active_turn.entries.clear();
        self.inserted_turns = 0;
        self.transcript_scroll = 0;
        self.agent_markdown_stream = None;
        self.clear_active_live_sections();
        self.pending_planning_suggestion = None;
        self.set_plan_approval_interaction(false);
        self.notice = Some("Cleared local transcript view.".into());
    }

    pub fn scroll_transcript(&mut self, delta: i32) {
        if delta < 0 {
            self.transcript_scroll = self
                .transcript_scroll
                .saturating_add(delta.unsigned_abs() as usize);
        } else {
            self.transcript_scroll = self.transcript_scroll.saturating_sub(delta as usize);
        }
    }

    pub fn set_runtime_phase(&mut self, phase: RuntimePhase, detail: Option<String>) {
        self.runtime_phase = phase;
        self.runtime_phase_detail = detail;
    }

    pub fn runtime_phase_label(&self) -> &'static str {
        match self.runtime_phase {
            RuntimePhase::Idle => "idle",
            RuntimePhase::LocalCommand => "local-command",
            RuntimePhase::SendingPrompt => "sending-prompt",
            RuntimePhase::ProcessingResponse => "processing-response",
            RuntimePhase::RunningTool => "running-tool",
            RuntimePhase::RebuildingBackend => "rebuilding-backend",
            RuntimePhase::BackendReady => "backend-ready",
            RuntimePhase::OAuthStarting => "oauth-starting",
            RuntimePhase::OAuthWaitingCallback => "oauth-waiting-callback",
            RuntimePhase::OAuthExchangingToken => "oauth-exchanging-token",
            RuntimePhase::OAuthSaved => "oauth-saved",
            RuntimePhase::Failed => "failed",
        }
    }

    pub fn remember_command(&mut self, command_name: &str) {
        self.recent_commands.retain(|value| value != command_name);
        self.recent_commands.insert(0, command_name.to_string());
        self.recent_commands.truncate(5);
    }

    pub fn open_overlay(&mut self, overlay: Overlay) {
        if matches!(overlay, Overlay::CommandPalette) {
            self.command_palette_idx = 0;
        }
        if matches!(overlay, Overlay::ProviderPicker) {
            self.provider_picker_idx = selected_provider_family_idx_for_config(&self.config);
        }
        if matches!(overlay, Overlay::ResumePicker) {
            self.resume_picker_idx = 0;
        }
        if matches!(overlay, Overlay::ModelPicker) {
            self.model_picker_idx = self.selected_preset_idx();
        }
        if matches!(overlay, Overlay::BaseUrlEditor) {
            self.base_url_input = self
                .config
                .base_url
                .clone()
                .unwrap_or_else(|| "http://localhost:11434".to_string());
        }
        if matches!(overlay, Overlay::ApiKeyEditor) {
            self.api_key_input.clear();
        }
        self.overlay = Some(overlay);
    }

    pub fn sync_command_palette_with_input(&mut self) {
        if input_requests_command_palette(self.input.as_str()) {
            if matches!(self.overlay, None | Some(Overlay::CommandPalette)) {
                self.open_overlay(Overlay::CommandPalette);
            }
        } else if matches!(self.overlay, Some(Overlay::CommandPalette)) {
            self.overlay = None;
        }
    }

    pub fn set_agent_execution_mode(&mut self, mode: AgentExecutionMode) {
        self.agent_execution_mode = mode;
    }

    pub fn agent_execution_mode_label(&self) -> &'static str {
        match self.agent_execution_mode {
            AgentExecutionMode::Execute => "execute",
            AgentExecutionMode::Plan => "plan",
        }
    }

    pub fn bash_approval_mode_label(&self) -> &'static str {
        match self.bash_approval_mode {
            BashApprovalMode::Always => "always",
            BashApprovalMode::Once => "once",
            BashApprovalMode::Suggestion => "suggestion",
        }
    }

    pub fn pending_question_option_label(&self, index: usize) -> Option<String> {
        self.pending_request_input()
            .and_then(|interaction| interaction.options.get(index))
            .map(|(label, _)| label.clone())
    }

    pub fn has_pending_approval(&self) -> bool {
        self.pending_command_approval().is_some()
    }

    pub fn has_pending_planning_suggestion(&self) -> bool {
        self.pending_planning_suggestion.is_some()
    }

    pub fn queue_planning_suggestion(&mut self, prompt: impl Into<String>) {
        self.pending_planning_suggestion = Some(prompt.into());
        self.notice = Some(
            "This looks like a non-trivial task. Enter planning mode first or continue in execute mode."
                .into(),
        );
        self.transcript_scroll = 0;
    }

    pub fn take_pending_planning_suggestion(&mut self) -> Option<String> {
        self.pending_planning_suggestion.take()
    }

    pub fn clear_pending_planning_suggestion(&mut self) {
        self.pending_planning_suggestion = None;
    }

    pub fn has_pending_plan_approval(&self) -> bool {
        self.pending_plan_approval_interaction().is_some()
    }

    pub fn active_pending_interaction(&self) -> Option<ActivePendingInteraction<'_>> {
        if let Some(snapshot) = self.pending_plan_approval_interaction() {
            return Some(ActivePendingInteraction {
                kind: ActivePendingInteractionKind::PlanApproval,
                _snapshot: snapshot,
            });
        }
        if let Some(snapshot) = self.pending_command_approval() {
            return Some(ActivePendingInteraction {
                kind: ActivePendingInteractionKind::ShellApproval,
                _snapshot: snapshot,
            });
        }
        if let Some(snapshot) = self.pending_request_input() {
            let kind = match snapshot.source.as_deref() {
                Some("plan_agent") => ActivePendingInteractionKind::PlanningQuestion,
                Some("explore_agent") => ActivePendingInteractionKind::ExplorationQuestion,
                Some(_) => ActivePendingInteractionKind::SubAgentQuestion,
                None => ActivePendingInteractionKind::RequestInput,
            };
            return Some(ActivePendingInteraction {
                kind,
                _snapshot: snapshot,
            });
        }
        None
    }

    pub fn set_pending_plan_approval(&mut self, pending: bool) {
        self.set_plan_approval_interaction(pending);
        self.persist_runtime_state();
    }

    pub fn pending_request_input(&self) -> Option<&PendingInteractionSnapshot> {
        self.snapshot
            .pending_interactions
            .iter()
            .find(|item| item.kind == InteractionKind::RequestInput)
    }

    pub fn has_local_pending_request_input(&self) -> bool {
        self.pending_request_input()
            .and_then(|item| item.source.as_deref())
            .is_some()
    }

    pub fn pending_command_approval(&self) -> Option<&PendingInteractionSnapshot> {
        self.snapshot
            .pending_interactions
            .iter()
            .find(|item| item.kind == InteractionKind::Approval)
    }

    pub fn pending_plan_approval_interaction(&self) -> Option<&PendingInteractionSnapshot> {
        self.snapshot
            .pending_interactions
            .iter()
            .find(|item| item.kind == InteractionKind::PlanApproval)
    }

    pub fn completed_interaction(
        &self,
        kind: InteractionKind,
    ) -> Option<&CompletedInteractionSnapshot> {
        self.snapshot
            .completed_interactions
            .iter()
            .find(|item| item.kind == kind)
    }

    pub fn record_completed_interaction(
        &mut self,
        kind: InteractionKind,
        title: impl Into<String>,
        summary: impl Into<String>,
        source: Option<String>,
    ) {
        let title = title.into();
        let summary = summary.into();
        self.snapshot
            .completed_interactions
            .retain(|item| item.kind != kind);
        self.snapshot
            .completed_interactions
            .push(CompletedInteractionSnapshot {
                kind,
                title: title.clone(),
                summary: summary.clone(),
                source: source.clone(),
            });
        self.ensure_completed_interaction_entry(kind, title.as_str(), summary.as_str(), source.as_deref());
        self.persist_runtime_state();
    }

    pub fn record_local_request_input(
        &mut self,
        source: impl Into<String>,
        title: impl Into<String>,
        options: Vec<(String, String)>,
        note: Option<String>,
    ) {
        self.snapshot.pending_interactions.retain(|item| {
            !(item.kind == InteractionKind::RequestInput && item.source.as_deref().is_some())
        });
        let title = title.into();
        self.snapshot
            .pending_interactions
            .push(PendingInteractionSnapshot {
                kind: InteractionKind::RequestInput,
                title: title.clone(),
                summary: note.clone().unwrap_or_default(),
                options,
                note,
                approval: None,
                source: Some(source.into()),
            });
        self.notice = Some(format!("{title}"));
        self.persist_runtime_state();
    }

    pub fn clear_local_request_input(&mut self) {
        self.snapshot.pending_interactions.retain(|item| {
            !(item.kind == InteractionKind::RequestInput && item.source.as_deref().is_some())
        });
        self.persist_runtime_state();
    }

    pub fn close_overlay(&mut self) {
        self.overlay = match self.overlay {
            Some(Overlay::BaseUrlEditor) => Some(Overlay::ModelPicker),
            Some(Overlay::ApiKeyEditor) => Some(Overlay::CodexAuthGuide),
            Some(Overlay::CodexAuthGuide) => Some(Overlay::ModelPicker),
            _ => None,
        };
    }

    pub fn has_any_transcript(&self) -> bool {
        !self.committed_turns.is_empty() || !self.active_turn.entries.is_empty()
    }

    pub fn transcript_entry_count(&self) -> usize {
        self.committed_turns
            .iter()
            .map(|turn| turn.entries.len())
            .sum::<usize>()
            + self.active_turn.entries.len()
    }

    pub fn committed_entry_count(&self) -> usize {
        self.committed_turns
            .iter()
            .map(|turn| turn.entries.len())
            .sum()
    }

    fn materialize_active_live_entries(&mut self) {
        let sections = [
            (
                "Exploring",
                self.active_live
                    .exploration_actions
                    .iter()
                    .chain(self.active_live.exploration_notes.iter())
                    .cloned()
                    .collect::<Vec<_>>(),
            ),
            (
                "Planning",
                self.active_live
                    .planning_actions
                    .iter()
                    .chain(self.active_live.planning_notes.iter())
                    .cloned()
                    .collect::<Vec<_>>(),
            ),
            (
                "Running",
                self.active_live.running_actions.to_vec(),
            ),
        ];

        for (role, lines) in sections {
            if lines.is_empty() {
                continue;
            }
            self.active_turn.entries.push(TranscriptEntry {
                role: role.to_string(),
                message: lines.join("\n"),
            });
        }
    }

    fn commit_active_turn(&mut self) {
        self.finalize_agent_stream(None);
        self.materialize_active_live_entries();
        if self.active_turn.entries.is_empty() {
            self.clear_active_live_sections();
            return;
        }
        let turn = std::mem::take(&mut self.active_turn);
        let ordinal = self.committed_turns.len();
        self.persist_turn(ordinal, &turn);
        self.committed_turns.push(turn);
        self.transcript_scroll = 0;
        self.clear_active_live_sections();
    }

    pub fn finalize_active_turn(&mut self) {
        self.commit_active_turn();
    }

    pub fn restore_committed_turns(&mut self, turns: Vec<TranscriptTurn>) {
        self.committed_turns = turns;
        self.active_turn.entries.clear();
        self.inserted_turns = 0;
        self.transcript_scroll = 0;
        self.agent_markdown_stream = None;
        self.clear_active_live_sections();
    }

    pub fn clear_active_live_sections(&mut self) {
        self.active_live = ActiveLiveSections::default();
    }

    pub fn record_exploration_action(&mut self, action: impl Into<String>) {
        let action = action.into();
        if !self
            .active_live
            .exploration_actions
            .iter()
            .any(|item| item == &action)
        {
            self.active_live.exploration_actions.push(action);
        }
    }

    pub fn record_exploration_note(&mut self, note: impl Into<String>) {
        let note = note.into();
        if !self
            .active_live
            .exploration_notes
            .iter()
            .any(|item| item == &note)
        {
            self.active_live.exploration_notes.push(note);
        }
    }

    pub fn record_running_action(&mut self, action: impl Into<String>) {
        let action = action.into();
        if !self
            .active_live
            .running_actions
            .iter()
            .any(|item| item == &action)
        {
            self.active_live.running_actions.push(action);
        }
    }

    pub fn record_planning_action(&mut self, action: impl Into<String>) {
        let action = action.into();
        if !self
            .active_live
            .planning_actions
            .iter()
            .any(|item| item == &action)
        {
            self.active_live.planning_actions.push(action);
        }
    }

    pub fn record_planning_note(&mut self, note: impl Into<String>) {
        let note = note.into();
        if !self
            .active_live
            .planning_notes
            .iter()
            .any(|item| item == &note)
        {
            self.active_live.planning_notes.push(note);
        }
    }

    pub fn attach_state_db(&mut self, state_db: Arc<StateDb>) {
        let status = state_db.path().display().to_string();
        self.recent_sessions = state_db.list_recent_sessions(20).unwrap_or_default();
        self.state_db = Some(state_db);
        self.state_db_status = Some(status);
        if !self.snapshot.session_id.is_empty() {
            self.persist_runtime_state();
        }
    }

    pub fn set_state_db_error(&mut self, error: String) {
        self.state_db = None;
        self.state_db_status = Some(state_db_status_error("unavailable", error));
    }

    fn persist_runtime_state(&mut self) {
        let Some(state_db) = self.state_db.as_ref() else {
            return;
        };
        if self.snapshot.session_id.is_empty() {
            return;
        }

        if let Err(err) = state_db.upsert_session(
            &self.snapshot.session_id,
            &self.snapshot.cwd,
            &self.snapshot.branch,
            &self.config.provider,
            self.current_model_label(),
            self.config.base_url.as_deref(),
            self.agent_execution_mode_label(),
            self.bash_approval_mode_label(),
            self.snapshot.plan_explanation.as_deref(),
            self.snapshot.history_len,
            self.transcript_entry_count(),
        ) {
            self.state_db_status = Some(state_db_status_error("write failed", err.to_string()));
            return;
        }

        let plan_steps = self
            .snapshot
            .plan_steps
            .iter()
            .enumerate()
            .map(|(step_index, (status, step))| PersistedPlanStep {
                step_index,
                status: status.clone(),
                step: step.clone(),
            })
            .collect::<Vec<_>>();
        if let Err(err) = state_db.replace_plan_steps(&self.snapshot.session_id, &plan_steps) {
            self.state_db_status =
                Some(state_db_status_error("plan write failed", err.to_string()));
            return;
        }

        let mut interactions = Vec::new();
        for interaction in &self.snapshot.pending_interactions {
            match interaction.kind {
                InteractionKind::RequestInput => {
                    let options_summary = interaction
                        .options
                        .iter()
                        .map(|(label, _)| label.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    let summary = match interaction.note.as_deref() {
                        Some(note) if !note.is_empty() => format!("{options_summary} | {note}"),
                        _ => options_summary,
                    };
                    interactions.push(PersistedInteraction {
                        kind: "request_input".to_string(),
                        status: "pending".to_string(),
                        title: interaction.title.clone(),
                        summary,
                        payload: Some(json!({
                            "question": interaction.title,
                            "options": interaction.options,
                            "note": interaction.note,
                            "source": interaction.source,
                        })),
                    });
                }
                InteractionKind::PlanApproval => {
                    interactions.push(PersistedInteraction {
                        kind: "plan_approval".to_string(),
                        status: "pending".to_string(),
                        title: interaction.title.clone(),
                        summary: interaction.summary.clone(),
                        payload: None,
                    });
                }
                InteractionKind::Approval => {
                    if let Some(approval) = interaction.approval.as_ref() {
                        interactions.push(PersistedInteraction {
                            kind: "approval".to_string(),
                            status: "pending".to_string(),
                            title: interaction.title.clone(),
                            summary: approval.command.clone(),
                            payload: Some(json!({
                                "tool_use_id": approval.tool_use_id,
                                "command": approval.command,
                                "allow_net": approval.allow_net,
                                "program": approval.payload.program,
                                "args": approval.payload.args,
                                "cwd": approval.payload.cwd,
                                "env": approval.payload.env,
                            })),
                        });
                    }
                }
            }
        }
        for interaction in &self.snapshot.completed_interactions {
            let kind = match interaction.kind {
                InteractionKind::RequestInput => "request_input",
                InteractionKind::Approval => "approval",
                InteractionKind::PlanApproval => "plan_approval",
            };
            interactions.push(PersistedInteraction {
                kind: kind.to_string(),
                status: "completed".to_string(),
                title: interaction.title.clone(),
                summary: interaction.summary.clone(),
                payload: None,
            });
        }

        if let Err(err) = state_db.replace_interactions(&self.snapshot.session_id, &interactions) {
            self.state_db_status = Some(state_db_status_error(
                "interaction write failed",
                err.to_string(),
            ));
            return;
        }

        self.recent_sessions = state_db.list_recent_sessions(20).unwrap_or_default();
        self.state_db_status = Some(state_db.path().display().to_string());
    }

    fn persist_turn(&mut self, ordinal: usize, turn: &TranscriptTurn) {
        let Some(state_db) = self.state_db.as_ref() else {
            return;
        };
        if self.snapshot.session_id.is_empty() {
            return;
        }
        let entries = turn
            .entries
            .iter()
            .map(|entry| PersistedTurnEntry {
                role: entry.role.clone(),
                message: entry.message.clone(),
            })
            .collect::<Vec<_>>();
        if let Err(err) = state_db.persist_turn(&self.snapshot.session_id, ordinal, &entries) {
            self.state_db_status =
                Some(state_db_status_error("turn write failed", err.to_string()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        input_requests_command_palette, state_db_status_error, ActivePendingInteractionKind,
        InteractionKind, PendingInteractionSnapshot, RuntimeSnapshot, TuiApp,
    };
    use crate::config::{ConfigManager, RaraConfig};
    use tempfile::tempdir;

    #[test]
    fn detects_slash_command_input() {
        assert!(input_requests_command_palette("/"));
        assert!(input_requests_command_palette("/help"));
        assert!(input_requests_command_palette("   /help"));
        assert!(!input_requests_command_palette(""));
        assert!(!input_requests_command_palette("help"));
        assert!(!input_requests_command_palette("   help"));
    }

    #[test]
    fn redacts_secrets_in_state_db_status_messages() {
        let rendered = state_db_status_error(
            "write failed",
            "token=supersecretvalue Authorization: Bearer abcdefghijklmnopqrstuvwxyz",
        );
        assert!(rendered.contains("write failed:"));
        assert!(rendered.contains("[REDACTED_SECRET]"));
        assert!(!rendered.contains("supersecretvalue"));
        assert!(!rendered.contains("abcdefghijklmnopqrstuvwxyz"));
    }

    #[test]
    fn prioritizes_active_pending_interaction_in_ui_order() {
        let dir = tempdir().expect("tempdir");
        let cm = ConfigManager {
            path: dir.path().join("config.json"),
        };
        let mut app = TuiApp::new(cm).expect("app");
        app.config = RaraConfig::default();
        app.snapshot = RuntimeSnapshot {
            pending_interactions: vec![
                PendingInteractionSnapshot {
                    kind: InteractionKind::RequestInput,
                    title: "Question".to_string(),
                    summary: String::new(),
                    options: Vec::new(),
                    note: None,
                    approval: None,
                    source: Some("plan_agent".to_string()),
                },
                PendingInteractionSnapshot {
                    kind: InteractionKind::Approval,
                    title: "Pending Approval".to_string(),
                    summary: "run cargo test".to_string(),
                    options: Vec::new(),
                    note: None,
                    approval: None,
                    source: None,
                },
                PendingInteractionSnapshot {
                    kind: InteractionKind::PlanApproval,
                    title: "Plan Ready".to_string(),
                    summary: "Review the plan.".to_string(),
                    options: Vec::new(),
                    note: None,
                    approval: None,
                    source: None,
                },
            ],
            ..RuntimeSnapshot::default()
        };

        let active = app
            .active_pending_interaction()
            .expect("pending interaction");
        assert_eq!(active.kind, ActivePendingInteractionKind::PlanApproval);
        assert_eq!(active._snapshot.title, "Plan Ready");
    }
}
