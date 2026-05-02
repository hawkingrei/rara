// The control-plane contract lands before ACP/Wire adapters are wired to it.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agent::AgentEvent;
use crate::tool::ToolOutputStream;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeControllerKind {
    LocalTui,
    LocalCli,
    Acp,
    Wire,
    AppServer,
    Runtime,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeProvenance {
    pub controller: RuntimeControllerKind,
    pub adapter: Option<String>,
    pub session_id: Option<String>,
    pub source_id: Option<String>,
    pub trusted: bool,
    pub user_provided: bool,
    pub generated: bool,
}

impl RuntimeProvenance {
    pub fn local_tui(session_id: impl Into<String>) -> Self {
        Self {
            controller: RuntimeControllerKind::LocalTui,
            adapter: None,
            session_id: Some(session_id.into()),
            source_id: None,
            trusted: true,
            user_provided: true,
            generated: false,
        }
    }

    pub fn protocol(
        controller: RuntimeControllerKind,
        adapter: impl Into<String>,
        session_id: Option<String>,
        source_id: Option<String>,
    ) -> Self {
        Self {
            controller,
            adapter: Some(adapter.into()),
            session_id,
            source_id,
            trusted: false,
            user_provided: true,
            generated: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeControlRequest {
    Session(SessionControlRequest),
    Input(InputControlRequest),
    Output(OutputSubscriptionRequest),
    PromptSource(PromptSourceControlRequest),
    SkillSource(SkillSourceControlRequest),
    Memory(MemoryControlRequest),
    Hook(HookControlRequest),
    Approval(ApprovalControlRequest),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeControlEnvelope {
    pub request_id: String,
    pub provenance: RuntimeProvenance,
    pub request: RuntimeControlRequest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionControlRequest {
    CreateSession,
    ResumeSession { session_id: String },
    CancelCurrentTurn,
    InterruptCurrentTurn,
    QueryRuntimeState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputControlRequest {
    SubmitUserPrompt { prompt: String },
    AnswerPendingInput { answer: String },
    AnswerPlanApproval { approved: bool },
    AnswerShellApproval { decision: ShellApprovalDecision },
    SubmitFollowUp { prompt: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShellApprovalDecision {
    Once,
    Prefix,
    Always,
    Suggestion,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputSubscriptionRequest {
    Subscribe { subscriber_id: String },
    Unsubscribe { subscriber_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptSourceLifetime {
    Turns(u32),
    Session,
    Persistent,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptSourceRegistration {
    pub source_id: String,
    pub scope: SourceScope,
    pub layer: SourceLayer,
    pub budget_hint_tokens: Option<u32>,
    pub lifetime: PromptSourceLifetime,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceScope {
    Home,
    Repo,
    CurrentWorkingDirectory,
    Session,
    Protocol,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceLayer {
    System,
    Developer,
    User,
    Memory,
    Skill,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptSourceControlRequest {
    Register(PromptSourceRegistration),
    Unregister { source_id: String },
    QuerySources,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkillSourceControlRequest {
    RegisterRoot {
        source_id: String,
        root: String,
        precedence_hint: Option<i32>,
    },
    RegisterSkill {
        source_id: String,
        name: String,
        content: String,
        precedence_hint: Option<i32>,
    },
    DisableSkill {
        name: String,
        source_id: Option<String>,
    },
    QuerySkills,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryControlRequest {
    AddRecord {
        memory_id: String,
        scope: MemoryScope,
        content: String,
        metadata: Value,
    },
    QueryMetadata,
    SelectionSnapshot,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryScope {
    Thread,
    Workspace,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HookControlRequest {
    Declare {
        hook_id: String,
        lifecycle: HookLifecycle,
        description: String,
    },
    QueryHooks,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HookLifecycle {
    SessionStart,
    UserPromptSubmit,
    PreToolUse,
    PostToolUse,
    Stop,
    PreCompact,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApprovalControlRequest {
    AnswerPendingApproval { approval_id: String, approved: bool },
    QueryPendingApprovals,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuntimeControlEvent {
    pub event_id: String,
    pub provenance: RuntimeProvenance,
    pub sequence: u64,
    pub event: RuntimeEvent,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum RuntimeEvent {
    Session(SessionEvent),
    Input(InputEvent),
    Assistant(AssistantEvent),
    Tool(ToolEvent),
    Approval(ApprovalEvent),
    Plan(PlanEvent),
    PromptSource(PromptSourceEvent),
    Skill(SkillEvent),
    Memory(MemoryEvent),
    Hook(HookEvent),
    Context(ContextEvent),
    Warning(WarningEvent),
    Error(ErrorEvent),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionEvent {
    Created { session_id: String },
    Resumed { session_id: String },
    Status { message: String },
    TurnStarted,
    TurnCancelled,
    TurnInterrupted,
    TurnFinished,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputEvent {
    UserPromptSubmitted,
    FollowUpQueued { queue_len: usize },
    PendingInputAnswered,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssistantEvent {
    Text(String),
    TextDelta(String),
    ThinkingDelta(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolStream {
    Stdout,
    Stderr,
}

impl From<ToolOutputStream> for ToolStream {
    fn from(stream: ToolOutputStream) -> Self {
        match stream {
            ToolOutputStream::Stdout => Self::Stdout,
            ToolOutputStream::Stderr => Self::Stderr,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ToolEvent {
    Use {
        name: String,
        input: Value,
    },
    Result {
        name: String,
        content: String,
        is_error: bool,
    },
    Progress {
        name: String,
        stream: ToolStream,
        chunk: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApprovalEvent {
    Requested { approval_id: String, kind: String },
    Answered { approval_id: String, approved: bool },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlanEvent {
    Updated,
    Approved,
    Continued,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptSourceEvent {
    Registered { source_id: String },
    Unregistered { source_id: String },
    Dropped { source_id: String, reason: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkillEvent {
    Registered { source_id: String, name: String },
    Shadowed { name: String, by_source_id: String },
    Failed { source_id: String, reason: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryEvent {
    RecordAdded { memory_id: String },
    SelectionUpdated,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HookEvent {
    Declared {
        hook_id: String,
        lifecycle: HookLifecycle,
    },
    Ignored {
        hook_id: String,
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContextEvent {
    SnapshotUpdated,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WarningEvent {
    RuntimeWarning { message: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorEvent {
    RuntimeError { message: String },
}

pub fn agent_event_to_runtime_event(event: AgentEvent) -> RuntimeEvent {
    match event {
        AgentEvent::Status(message) => RuntimeEvent::Session(SessionEvent::Status { message }),
        AgentEvent::AssistantText(text) => RuntimeEvent::Assistant(AssistantEvent::Text(text)),
        AgentEvent::AssistantDelta(delta) => {
            RuntimeEvent::Assistant(AssistantEvent::TextDelta(delta))
        }
        AgentEvent::AssistantThinkingDelta(delta) => {
            RuntimeEvent::Assistant(AssistantEvent::ThinkingDelta(delta))
        }
        AgentEvent::ToolUse { name, input } => RuntimeEvent::Tool(ToolEvent::Use { name, input }),
        AgentEvent::ToolResult {
            name,
            content,
            is_error,
        } => RuntimeEvent::Tool(ToolEvent::Result {
            name,
            content,
            is_error,
        }),
        AgentEvent::ToolProgress {
            name,
            stream,
            chunk,
        } => RuntimeEvent::Tool(ToolEvent::Progress {
            name,
            stream: stream.into(),
            chunk,
        }),
    }
}

pub fn wrap_agent_event(
    event_id: impl Into<String>,
    sequence: u64,
    provenance: RuntimeProvenance,
    event: AgentEvent,
) -> RuntimeControlEvent {
    RuntimeControlEvent {
        event_id: event_id.into(),
        provenance,
        sequence,
        event: agent_event_to_runtime_event(event),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn agent_tool_progress_maps_to_structured_runtime_event() {
        let event = agent_event_to_runtime_event(AgentEvent::ToolProgress {
            name: "bash".to_string(),
            stream: ToolOutputStream::Stderr,
            chunk: "error\n".to_string(),
        });

        assert_eq!(
            event,
            RuntimeEvent::Tool(ToolEvent::Progress {
                name: "bash".to_string(),
                stream: ToolStream::Stderr,
                chunk: "error\n".to_string(),
            })
        );
    }

    #[test]
    fn agent_status_maps_to_session_status_not_warning() {
        let event = agent_event_to_runtime_event(AgentEvent::Status("Sending prompt.".to_string()));

        assert_eq!(
            event,
            RuntimeEvent::Session(SessionEvent::Status {
                message: "Sending prompt.".to_string()
            })
        );
    }

    #[test]
    fn prompt_source_lifetime_serializes_as_turn_based_contract() {
        let request = RuntimeControlEnvelope {
            request_id: "req-1".to_string(),
            provenance: RuntimeProvenance::protocol(
                RuntimeControllerKind::Acp,
                "acp",
                Some("session-1".to_string()),
                Some("source-1".to_string()),
            ),
            request: RuntimeControlRequest::PromptSource(PromptSourceControlRequest::Register(
                PromptSourceRegistration {
                    source_id: "source-1".to_string(),
                    scope: SourceScope::Protocol,
                    layer: SourceLayer::User,
                    budget_hint_tokens: Some(256),
                    lifetime: PromptSourceLifetime::Turns(2),
                    content: "adapter context".to_string(),
                },
            )),
        };

        let value = serde_json::to_value(&request).unwrap();

        assert_eq!(value["request_id"], json!("req-1"));
        assert_eq!(value["provenance"]["controller"], json!("Acp"));
        assert_eq!(
            value["request"]["PromptSource"]["Register"]["lifetime"],
            json!({ "Turns": 2 })
        );
    }

    #[test]
    fn wrapped_agent_event_preserves_provenance_and_sequence() {
        let control_event = wrap_agent_event(
            "evt-1",
            42,
            RuntimeProvenance::local_tui("session-1"),
            AgentEvent::AssistantDelta("hello".to_string()),
        );

        assert_eq!(control_event.event_id, "evt-1");
        assert_eq!(control_event.sequence, 42);
        assert_eq!(
            control_event.provenance.controller,
            RuntimeControllerKind::LocalTui
        );
        assert_eq!(
            control_event.provenance.session_id.as_deref(),
            Some("session-1")
        );
        assert_eq!(
            control_event.event,
            RuntimeEvent::Assistant(AssistantEvent::TextDelta("hello".to_string()))
        );
    }
}
