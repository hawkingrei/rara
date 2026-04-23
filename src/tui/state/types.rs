use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use ratatui::text::Line;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::JoinHandle;

use super::super::markdown_stream::MarkdownStreamCollector;
use crate::agent::{Agent, AgentExecutionMode, BashApprovalMode};
use crate::config::{ConfigManager, RaraConfig};
use crate::state_db::{PersistedSessionSummary, StateDb};

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
    pub pending_question: Option<(String, Vec<(String, String)>, Option<String>)>,
    pub pending_approval: Option<PendingApprovalSnapshot>,
    pub pending_plan_approval: bool,
    pub completed_question: Option<(String, String)>,
    pub completed_approval: Option<(String, String)>,
    pub prompt_base_kind: String,
    pub prompt_section_keys: Vec<String>,
    pub prompt_source_status_lines: Vec<String>,
    pub prompt_warnings: Vec<String>,
}

#[derive(Default, Clone)]
pub struct PendingApprovalSnapshot {
    pub tool_use_id: String,
    pub command: String,
    pub allow_net: bool,
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
    pub(crate) raw_text: String,
    collector: MarkdownStreamCollector,
    committed_lines: Vec<Line<'static>>,
    pub(crate) display_lines: Vec<Line<'static>>,
}

impl AgentMarkdownStreamState {
    pub(crate) fn new(cwd: PathBuf) -> Self {
        Self {
            raw_text: String::new(),
            collector: MarkdownStreamCollector::new(None, &cwd),
            committed_lines: Vec::new(),
            display_lines: Vec::new(),
        }
    }

    pub(crate) fn push_delta(&mut self, delta: &str) {
        self.raw_text.push_str(delta);
        self.collector.push_delta(delta);
        self.committed_lines
            .extend(self.collector.commit_complete_lines());
        self.display_lines = self.committed_lines.clone();
        self.display_lines.extend(self.collector.preview_lines());
    }
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
    pub pending_plan_approval: bool,
    pub terminal_focused: bool,
    pub state_db: Option<Arc<StateDb>>,
    pub state_db_status: Option<String>,
    pub running_task: Option<RunningTask>,
}
