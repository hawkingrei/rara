use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use ratatui::text::Line;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::JoinHandle;

use super::super::markdown_stream::MarkdownStreamCollector;
use super::super::queued_input::PendingFollowUpMessage;
use crate::agent::{Agent, AgentExecutionMode, BashApprovalMode};
use crate::codex_model_catalog::CodexModelOption;
use crate::config::{ConfigManager, RaraConfig};
use crate::context::{CompactionSourceContextEntry, PromptSourceContextEntry};
use crate::context::{RetrievalSelectedItemContextEntry, RetrievalSourceContextEntry};
use crate::oauth::SavedCodexAuthMode;
use crate::state_db::StateDb;
use crate::thread_store::ThreadSummary;
use crate::tool::ToolOutputStream;
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
    Context,
    Setup,
    ProviderPicker,
    ModelPicker,
    ResumePicker,
    BaseUrlEditor,
    AuthModePicker,
    ApiKeyEditor,
    ModelNameEditor,
    ReasoningEffortPicker,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProviderFamily {
    Codex,
    OpenAiCompatible,
    CandleLocal,
    Ollama,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LocalCommandKind {
    Help,
    Status,
    Context,
    Clear,
    Resume,
    Plan,
    Approval,
    Compact,
    Setup,
    Model,
    BaseUrl,
    Login,
    Logout,
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
    OAuthDeviceCodePrompt,
    OAuthPollingDeviceCode,
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
    pub last_compaction_recent_files: Vec<String>,
    pub last_compaction_boundary_version: Option<u32>,
    pub last_compaction_boundary_before_tokens: Option<usize>,
    pub last_compaction_boundary_recent_file_count: Option<usize>,
    pub compaction_source_entries: Vec<CompactionSourceContextEntry>,
    pub plan_steps: Vec<(String, String)>,
    pub plan_explanation: Option<String>,
    pub pending_interactions: Vec<PendingInteractionSnapshot>,
    pub completed_interactions: Vec<CompletedInteractionSnapshot>,
    pub prompt_base_kind: String,
    pub prompt_section_keys: Vec<String>,
    pub prompt_source_entries: Vec<PromptSourceContextEntry>,
    pub prompt_source_status_lines: Vec<String>,
    pub prompt_append_system_prompt: Option<String>,
    pub prompt_warnings: Vec<String>,
    pub retrieval_source_entries: Vec<RetrievalSourceContextEntry>,
    pub retrieval_selected_items: Vec<RetrievalSelectedItemContextEntry>,
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

pub enum TaskKind {
    Query,
    Compact,
    Rebuild,
    OAuth,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OAuthLoginMode {
    Browser,
    DeviceCode,
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
        result: anyhow::Result<RebuildSuccess>,
    },
    OAuth {
        mode: OAuthLoginMode,
        result: anyhow::Result<secrecy::SecretString>,
    },
}

pub struct RebuildSuccess {
    pub agent: Agent,
    pub warnings: Vec<String>,
}

pub enum TuiEvent {
    Transcript {
        role: &'static str,
        message: String,
    },
    ToolProgress {
        name: String,
        stream: ToolOutputStream,
        chunk: String,
    },
}

pub struct RunningTask {
    pub kind: TaskKind,
    pub receiver: UnboundedReceiver<TuiEvent>,
    pub handle: JoinHandle<TaskCompletion>,
    pub started_at: Instant,
    pub next_heartbeat_after_secs: u64,
}

pub const PROVIDER_FAMILIES: [(ProviderFamily, &str, &str); 4] = [
    (
        ProviderFamily::Codex,
        "Codex",
        "Use the Codex-compatible API with browser login, device-code login, or an API key.",
    ),
    (
        ProviderFamily::OpenAiCompatible,
        "OpenAI-compatible",
        "Use any OpenAI-compatible endpoint with a custom base URL, model name, and API key.",
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

#[derive(Default)]
pub(crate) struct CommittedTranscriptRenderCache {
    pub generation: u64,
    pub width: u16,
    pub lines: Vec<Line<'static>>,
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
    pub reasoning_effort_picker_idx: usize,
    pub auth_mode_idx: usize,
    pub command_palette_idx: usize,
    pub base_url_input: String,
    pub api_key_input: String,
    pub model_name_input: String,
    pub codex_model_options: Vec<CodexModelOption>,
    pub recent_commands: Vec<String>,
    pub recent_threads: Vec<ThreadSummary>,
    pub resume_picker_idx: usize,
    pub committed_render_generation: u64,
    pub committed_render_cache: RefCell<CommittedTranscriptRenderCache>,
    pub transcript_scroll: usize,
    pub agent_markdown_stream: Option<AgentMarkdownStreamState>,
    pub active_live: ActiveLiveSections,
    pub pending_planning_suggestion: Option<String>,
    pub pending_follow_up_messages: Vec<PendingFollowUpMessage>,
    pub queued_follow_up_messages: Vec<String>,
    pub running_tool_boundary_count: u64,
    pub terminal_focused: bool,
    pub state_db: Option<Arc<StateDb>>,
    pub state_db_status: Option<String>,
    pub running_task: Option<RunningTask>,
    pub repo_context_task: Option<JoinHandle<(Option<String>, Option<String>)>>,
    pub repo_slug: Option<String>,
    pub current_pr_url: Option<String>,
    pub codex_auth_mode: Option<SavedCodexAuthMode>,
}
