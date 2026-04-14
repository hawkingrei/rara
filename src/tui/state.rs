use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::JoinHandle;
use std::time::Instant;
use std::sync::Arc;
use serde_json::json;

use crate::agent::{Agent, AgentExecutionMode, BashApprovalMode};
use crate::config::{ConfigManager, RaraConfig};
use crate::state_db::{PersistedInteraction, PersistedPlanStep, PersistedSessionSummary, PersistedTurnEntry, StateDb};

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
    Search,
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
    pub plan_steps: Vec<(String, String)>,
    pub plan_explanation: Option<String>,
    pub pending_question: Option<(String, Vec<(String, String)>, Option<String>)>,
    pub pending_approval: Option<PendingApprovalSnapshot>,
    pub completed_question: Option<(String, String)>,
    pub completed_approval: Option<(String, String)>,
}

#[derive(Default, Clone)]
pub struct PendingApprovalSnapshot {
    pub tool_use_id: String,
    pub command: String,
    pub allow_net: bool,
}

pub enum TaskKind {
    Query,
    Rebuild,
    OAuth,
}

pub enum TaskCompletion {
    Query {
        agent: Agent,
        result: anyhow::Result<()>,
    },
    Rebuild {
        result: anyhow::Result<Agent>,
    },
    OAuth {
        result: anyhow::Result<String>,
    },
}

pub enum TuiEvent {
    Transcript {
        role: &'static str,
        message: String,
    },
}

pub struct RunningTask {
    pub kind: TaskKind,
    pub receiver: UnboundedReceiver<TuiEvent>,
    pub handle: JoinHandle<TaskCompletion>,
    pub started_at: Instant,
    pub next_heartbeat_after_secs: u64,
}

pub const CODEX_MODEL_PRESETS: [(&str, &str, &str); 2] = [
    ("Codex (OAuth)", "codex", "codex"),
    ("Codex (API Key)", "codex", "codex"),
];

pub const LOCAL_MODEL_PRESETS: [(&str, &str, &str); 3] = [
    ("Gemma 4 E4B (Experimental)", "gemma4", "gemma4-e4b"),
    ("Gemma 4 E2B (Experimental)", "gemma4", "gemma4-e2b"),
    ("Qwn3 8B", "qwn3", "qwn3-8b"),
];

pub const OLLAMA_MODEL_PRESETS: [(&str, &str, &str); 3] = [
    ("Gemma 4", "ollama", "gemma4"),
    ("Gemma 4 E4B", "ollama", "gemma4:e4b"),
    ("Gemma 4 E2B", "ollama", "gemma4:e2b"),
];

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
    pub terminal_focused: bool,
    pub state_db: Option<Arc<StateDb>>,
    pub state_db_status: Option<String>,
    pub running_task: Option<RunningTask>,
}

impl TuiApp {
    pub fn new(cm: ConfigManager) -> Self {
        let cfg = cm.load();
        let overlay = if cfg.api_key.is_none() && super::provider_requires_api_key(&cfg.provider) {
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
        Self {
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
            terminal_focused: true,
            state_db: None,
            state_db_status: None,
            running_task: None,
        }
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
        let next = (self.selected_preset_idx() + 1) % current_model_presets(self.provider_picker_idx).len();
        self.select_local_model(next);
    }

    pub fn sync_snapshot(&mut self, agent: &Agent) {
        let (cwd, branch) = agent.workspace.get_env_info();
        self.snapshot = RuntimeSnapshot {
            cwd,
            branch,
            session_id: agent.session_id.clone(),
            history_len: agent.history.len(),
            total_input_tokens: agent.total_input_tokens,
            total_output_tokens: agent.total_output_tokens,
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
            pending_question: agent.pending_user_input.as_ref().map(|question| {
                (
                    question.question.clone(),
                    question.options.clone(),
                    question.note.clone(),
                )
            }),
            pending_approval: agent.pending_approval.as_ref().map(|pending| PendingApprovalSnapshot {
                tool_use_id: pending.tool_use_id.clone(),
                command: pending.command.clone(),
                allow_net: pending.allow_net,
            }),
            completed_question: agent
                .completed_user_input
                .as_ref()
                .map(|item| (item.title.clone(), item.summary.clone())),
            completed_approval: agent
                .completed_approval
                .as_ref()
                .map(|item| (item.title.clone(), item.summary.clone())),
        };
        self.agent_execution_mode = agent.execution_mode;
        self.bash_approval_mode = agent.bash_approval_mode;
        self.persist_runtime_state();
    }

    pub fn push_entry(&mut self, role: &'static str, message: impl Into<String>) {
        if role == "You" && !self.active_turn.entries.is_empty() {
            self.commit_active_turn();
        }
        self.active_turn.entries.push(TranscriptEntry {
            role: role.to_string(),
            message: message.into(),
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

    pub fn push_notice(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.notice = Some(message.clone());
        self.push_entry("System", message);
    }

    pub fn reset_transcript(&mut self) {
        self.committed_turns.clear();
        self.active_turn.entries.clear();
        self.inserted_turns = 0;
        self.transcript_scroll = 0;
        self.notice = Some("Cleared local transcript view.".into());
    }

    pub fn scroll_transcript(&mut self, delta: i32) {
        if delta < 0 {
            self.transcript_scroll = self
                .transcript_scroll
                .saturating_add(delta.unsigned_abs() as usize);
        } else {
            self.transcript_scroll = self
                .transcript_scroll
                .saturating_sub(delta as usize);
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
            self.api_key_input = self.config.api_key.clone().unwrap_or_default();
        }
        self.overlay = Some(overlay);
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
        self.snapshot
            .pending_question
            .as_ref()
            .and_then(|(_, options, _)| options.get(index))
            .map(|(label, _)| label.clone())
    }

    pub fn has_pending_approval(&self) -> bool {
        self.snapshot.pending_approval.is_some()
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

    fn commit_active_turn(&mut self) {
        if self.active_turn.entries.is_empty() {
            return;
        }
        let turn = std::mem::take(&mut self.active_turn);
        let ordinal = self.committed_turns.len();
        self.persist_turn(ordinal, &turn);
        self.committed_turns.push(turn);
        self.transcript_scroll = 0;
    }

    pub fn finalize_active_turn(&mut self) {
        self.commit_active_turn();
    }

    pub fn restore_committed_turns(&mut self, turns: Vec<TranscriptTurn>) {
        self.committed_turns = turns;
        self.active_turn.entries.clear();
        self.inserted_turns = 0;
        self.transcript_scroll = 0;
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
        self.state_db_status = Some(format!("unavailable: {error}"));
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
            self.state_db_status = Some(format!("write failed: {err}"));
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
            self.state_db_status = Some(format!("plan write failed: {err}"));
            return;
        }

        let mut interactions = Vec::new();
        if let Some((question, options, note)) = self.snapshot.pending_question.as_ref() {
            let options_summary = options
                .iter()
                .map(|(label, _)| label.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let summary = match note {
                Some(note) if !note.is_empty() => format!("{options_summary} | {note}"),
                _ => options_summary,
            };
            interactions.push(PersistedInteraction {
                kind: "request_input".to_string(),
                status: "pending".to_string(),
                title: question.clone(),
                summary,
                payload: Some(json!({
                    "question": question,
                    "options": options,
                    "note": note,
                })),
            });
        }
        if let Some(approval) = self.snapshot.pending_approval.as_ref() {
            interactions.push(PersistedInteraction {
                kind: "approval".to_string(),
                status: "pending".to_string(),
                title: "Pending Approval".to_string(),
                summary: approval.command.clone(),
                payload: Some(json!({
                    "tool_use_id": approval.tool_use_id,
                    "command": approval.command,
                    "allow_net": approval.allow_net,
                })),
            });
        }
        if let Some((title, summary)) = self.snapshot.completed_question.as_ref() {
            interactions.push(PersistedInteraction {
                kind: "request_input".to_string(),
                status: "completed".to_string(),
                title: title.clone(),
                summary: summary.clone(),
                payload: None,
            });
        }
        if let Some((title, summary)) = self.snapshot.completed_approval.as_ref() {
            interactions.push(PersistedInteraction {
                kind: "approval".to_string(),
                status: "completed".to_string(),
                title: title.clone(),
                summary: summary.clone(),
                payload: None,
            });
        }

        if let Err(err) = state_db.replace_interactions(&self.snapshot.session_id, &interactions) {
            self.state_db_status = Some(format!("interaction write failed: {err}"));
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
            self.state_db_status = Some(format!("turn write failed: {err}"));
        }
    }
}

pub fn selected_provider_family_idx_for_config(config: &RaraConfig) -> usize {
    match config.provider.as_str() {
        "codex" => 0,
        "ollama" => 2,
        _ => 1,
    }
}

pub fn current_model_presets(provider_picker_idx: usize) -> &'static [(&'static str, &'static str, &'static str)] {
    match PROVIDER_FAMILIES[provider_picker_idx].0 {
        ProviderFamily::Codex => &CODEX_MODEL_PRESETS,
        ProviderFamily::CandleLocal => &LOCAL_MODEL_PRESETS,
        ProviderFamily::Ollama => &OLLAMA_MODEL_PRESETS,
    }
}

pub fn selected_preset_idx_for_config(config: &RaraConfig, provider_picker_idx: usize) -> usize {
    current_model_presets(provider_picker_idx)
        .iter()
        .position(|(_, provider, model)| {
            config.provider == *provider && config.model.as_deref() == Some(*model)
        })
        .unwrap_or(0)
}
