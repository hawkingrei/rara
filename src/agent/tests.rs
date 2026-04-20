use super::planning::{
    parse_plan_block, parse_request_user_input_block, strip_continue_inspection_control,
};
use super::{
    Agent, AgentExecutionMode, AnthropicResponse, ContentBlock, Message, PendingUserInput,
    PlanStep, PlanStepStatus, RuntimeContinuationPhase, TokenUsage,
};
use crate::llm::LlmBackend;
use crate::session::SessionManager;
use crate::tool::{Tool, ToolError, ToolManager};
use crate::tool_result::ToolResultStore;
use crate::vectordb::VectorDB;
use crate::workspace::WorkspaceMemory;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tempfile::tempdir;

struct StubTool;

#[async_trait]
impl Tool for StubTool {
    fn name(&self) -> &str {
        "stub_tool"
    }
    fn description(&self) -> &str {
        "Return a simple structured result"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object"})
    }
    async fn call(&self, _input: Value) -> Result<Value, ToolError> {
        Ok(json!({ "status": "ok", "value": 42 }))
    }
}

struct ListFilesStub;

#[async_trait]
impl Tool for ListFilesStub {
    fn name(&self) -> &str {
        "list_files"
    }
    fn description(&self) -> &str {
        "Return a simple list result"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object"})
    }
    async fn call(&self, _input: Value) -> Result<Value, ToolError> {
        Ok(json!({ "path": ".", "entries": [] }))
    }
}

struct PlanAgentStub;

#[async_trait]
impl Tool for PlanAgentStub {
    fn name(&self) -> &str {
        "plan_agent"
    }
    fn description(&self) -> &str {
        "Return a delegated planning result"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object"})
    }
    async fn call(&self, _input: Value) -> Result<Value, ToolError> {
        Ok(json!({ "status": "ok", "summary": "delegated inspection complete" }))
    }
}

struct SequencedBackend {
    responses: Mutex<Vec<AnthropicResponse>>,
    observed_messages: Mutex<Vec<Vec<Message>>>,
    observed_tools: Mutex<Vec<Vec<String>>>,
}

impl SequencedBackend {
    fn new(responses: Vec<AnthropicResponse>) -> Self {
        Self {
            responses: Mutex::new(responses),
            observed_messages: Mutex::new(Vec::new()),
            observed_tools: Mutex::new(Vec::new()),
        }
    }

    fn observed_tools(&self) -> Vec<Vec<String>> {
        self.observed_tools.lock().expect("lock").clone()
    }
}

fn test_runtime_storage() -> (
    tempfile::TempDir,
    Arc<SessionManager>,
    Arc<WorkspaceMemory>,
    std::path::PathBuf,
) {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().to_path_buf();
    let rara_dir = root.join(".rara");
    std::fs::create_dir_all(rara_dir.join("rollouts")).expect("rollouts");
    std::fs::create_dir_all(rara_dir.join("sessions")).expect("sessions");
    std::fs::create_dir_all(rara_dir.join("tool-results")).expect("tool results");
    let session_manager = Arc::new(SessionManager {
        storage_dir: rara_dir.join("rollouts"),
        legacy_storage_dir: rara_dir.join("sessions"),
    });
    let workspace = Arc::new(WorkspaceMemory::from_paths(root, rara_dir.clone()));
    (temp, session_manager, workspace, rara_dir)
}

#[async_trait]
impl LlmBackend for SequencedBackend {
    async fn ask(&self, messages: &[Message], tools: &[Value]) -> Result<AnthropicResponse> {
        self.observed_messages
            .lock()
            .expect("lock")
            .push(messages.to_vec());
        self.observed_tools.lock().expect("lock").push(
            tools
                .iter()
                .filter_map(|tool| tool.get("name").and_then(Value::as_str).map(str::to_string))
                .collect(),
        );
        let mut responses = self.responses.lock().expect("lock");
        assert!(
            !responses.is_empty(),
            "test backend ran out of scripted responses"
        );
        Ok(responses.remove(0))
    }

    async fn embed(&self, _text: &str) -> Result<Vec<f32>> {
        Ok(vec![0.0; 8])
    }

    async fn summarize(&self, _messages: &[Message], _instruction: &str) -> Result<String> {
        Ok("summary".to_string())
    }
}

#[tokio::test]
async fn appends_continuation_after_tool_result() {
    let backend = Arc::new(SequencedBackend::new(vec![
        AnthropicResponse {
            content: vec![ContentBlock::ToolUse {
                id: "tool-1".to_string(),
                name: "stub_tool".to_string(),
                input: json!({}),
            }],
            stop_reason: Some("tool_use".to_string()),
            usage: Some(TokenUsage::default()),
        },
        AnthropicResponse {
            content: vec![ContentBlock::Text {
                text: "done".to_string(),
            }],
            stop_reason: Some("end_turn".to_string()),
            usage: Some(TokenUsage::default()),
        },
    ]));

    let mut tool_manager = ToolManager::new();
    tool_manager.register(Box::new(StubTool));
    let mut agent = Agent::new(
        tool_manager,
        backend.clone(),
        Arc::new(VectorDB::new("data/lancedb")),
        Arc::new(SessionManager::new().expect("session manager")),
        Arc::new(WorkspaceMemory::new().expect("workspace memory")),
    );

    agent
        .query_with_mode("do work".to_string(), super::AgentOutputMode::Silent)
        .await
        .expect("query should succeed");

    let observed = backend.observed_messages.lock().expect("lock");
    assert_eq!(observed.len(), 2);
    let second_round = &observed[1];
    let continuation =
        agent.runtime_continuation_message(RuntimeContinuationPhase::ToolResultsAvailable, 1);
    assert!(second_round
        .iter()
        .any(|message| message.content == continuation.content));
    assert!(second_round
        .iter()
        .any(|message| { message.content.to_string().contains("tool_result") }));
}

#[tokio::test]
async fn resumes_after_plan_approval_via_structured_continuation() {
    let (_temp, session_manager, workspace, rara_dir) = test_runtime_storage();
    let backend = Arc::new(SequencedBackend::new(vec![AnthropicResponse {
        content: vec![ContentBlock::Text {
            text: "Implemented the first plan step.".to_string(),
        }],
        stop_reason: Some("end_turn".to_string()),
        usage: Some(TokenUsage::default()),
    }]));

    let mut agent = Agent::new(
        ToolManager::new(),
        backend.clone(),
        Arc::new(VectorDB::new("data/lancedb")),
        session_manager,
        workspace,
    );
    agent.tool_result_store =
        ToolResultStore::new(rara_dir.join("tool-results")).expect("tool result store");
    agent.set_execution_mode(AgentExecutionMode::Plan);
    agent.current_plan = vec![PlanStep {
        step: "Modify workspace instruction discovery".to_string(),
        status: PlanStepStatus::Pending,
    }];

    agent
        .resume_after_plan_approval_with_events(false, super::AgentOutputMode::Silent, |_| {})
        .await
        .expect("resume should succeed");

    let observed = backend.observed_messages.lock().expect("lock");
    assert_eq!(observed.len(), 1);
    let runtime_texts = observed[0]
        .iter()
        .filter_map(|message| message.content.as_array())
        .flat_map(|blocks| blocks.iter())
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert!(runtime_texts
        .iter()
        .any(|text| text.contains("\"phase\": \"plan_approved\"")));
    assert!(runtime_texts
        .iter()
        .any(|text| text.contains("\"mode\": \"execute\"")));
    assert!(!runtime_texts.iter().any(
        |text| text.contains("Implement the approved plan using the current repository state")
    ));
}

#[tokio::test]
async fn does_not_append_continuation_without_tools() {
    let backend = Arc::new(SequencedBackend::new(vec![AnthropicResponse {
        content: vec![ContentBlock::Text {
            text: "final".to_string(),
        }],
        stop_reason: Some("end_turn".to_string()),
        usage: Some(TokenUsage::default()),
    }]));

    let mut agent = Agent::new(
        ToolManager::new(),
        backend.clone(),
        Arc::new(VectorDB::new("data/lancedb")),
        Arc::new(SessionManager::new().expect("session manager")),
        Arc::new(WorkspaceMemory::new().expect("workspace memory")),
    );

    agent
        .query_with_mode("hello".to_string(), super::AgentOutputMode::Silent)
        .await
        .expect("query should succeed");

    let observed = backend.observed_messages.lock().expect("lock");
    assert_eq!(observed.len(), 1);
    assert!(!agent.history.iter().any(|message| message
        .content
        .to_string()
        .contains("\"phase\": \"tool_results_available\"")));
}

#[tokio::test]
async fn errors_when_tool_loop_exceeds_limit() {
    let responses = (0..=super::MAX_TOOL_ROUNDS_PER_TURN)
        .map(|idx| AnthropicResponse {
            content: vec![ContentBlock::ToolUse {
                id: format!("tool-{idx}"),
                name: "stub_tool".to_string(),
                input: json!({}),
            }],
            stop_reason: Some("tool_use".to_string()),
            usage: Some(TokenUsage::default()),
        })
        .collect::<Vec<_>>();
    let backend = Arc::new(SequencedBackend::new(responses));

    let mut tool_manager = ToolManager::new();
    tool_manager.register(Box::new(StubTool));
    let mut agent = Agent::new(
        tool_manager,
        backend,
        Arc::new(VectorDB::new("data/lancedb")),
        Arc::new(SessionManager::new().expect("session manager")),
        Arc::new(WorkspaceMemory::new().expect("workspace memory")),
    );

    let error = agent
        .query_with_mode("loop".to_string(), super::AgentOutputMode::Silent)
        .await
        .expect_err("query should fail");
    assert!(error.to_string().contains("Tool loop exceeded"));
}

#[tokio::test]
async fn plan_mode_filters_write_tools_from_schema() {
    let backend = Arc::new(SequencedBackend::new(vec![AnthropicResponse {
        content: vec![ContentBlock::Text {
            text: "<plan>\n- [pending] Review the current project structure\n</plan>".to_string(),
        }],
        stop_reason: Some("end_turn".to_string()),
        usage: Some(TokenUsage::default()),
    }]));

    let mut tool_manager = ToolManager::new();
    tool_manager.register(Box::new(StubTool));
    tool_manager.register(Box::new(crate::tools::file::WriteFileTool));
    let mut agent = Agent::new(
        tool_manager,
        backend.clone(),
        Arc::new(VectorDB::new("data/lancedb")),
        Arc::new(SessionManager::new().expect("session manager")),
        Arc::new(WorkspaceMemory::new().expect("workspace memory")),
    );
    agent.set_execution_mode(AgentExecutionMode::Plan);

    agent
        .query_with_mode(
            "review-current-project".to_string(),
            super::AgentOutputMode::Silent,
        )
        .await
        .expect("query should succeed");

    let observed_tools = backend.observed_tools();
    assert_eq!(observed_tools.len(), 1);
    assert_eq!(observed_tools[0], vec!["stub_tool".to_string()]);
}

#[tokio::test]
async fn accepts_shallow_initial_plan_as_structured_outcome() {
    let backend = Arc::new(SequencedBackend::new(vec![AnthropicResponse {
        content: vec![ContentBlock::Text {
            text: "<plan>\n- [pending] Inspect the repository structure\n</plan>\nStart with the top-level layout.".to_string(),
        }],
        stop_reason: Some("end_turn".to_string()),
        usage: Some(TokenUsage::default()),
    }]));

    let mut tool_manager = ToolManager::new();
    tool_manager.register(Box::new(StubTool));
    let mut agent = Agent::new(
        tool_manager,
        backend.clone(),
        Arc::new(VectorDB::new("data/lancedb")),
        Arc::new(SessionManager::new().expect("session manager")),
        Arc::new(WorkspaceMemory::new().expect("workspace memory")),
    );
    agent.set_execution_mode(AgentExecutionMode::Plan);

    agent
        .query_with_mode("inspect".to_string(), super::AgentOutputMode::Silent)
        .await
        .expect("query should succeed");

    let observed = backend.observed_messages.lock().expect("lock");
    assert_eq!(observed.len(), 1);
    assert!(!agent.history.iter().any(|message| message
        .content
        .to_string()
        .contains("plan_continuation_required")));
}

#[tokio::test]
async fn last_query_plan_updated_tracks_only_the_final_planning_turn() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().to_path_buf();
    let rara_dir = root.join(".rara");
    std::fs::create_dir_all(rara_dir.join("rollouts")).expect("rollouts");
    std::fs::create_dir_all(rara_dir.join("sessions")).expect("sessions");
    let session_manager = Arc::new(SessionManager {
        storage_dir: rara_dir.join("rollouts"),
        legacy_storage_dir: rara_dir.join("sessions"),
    });
    let workspace = Arc::new(WorkspaceMemory::from_paths(root, rara_dir.clone()));

    let backend = Arc::new(SequencedBackend::new(vec![
        AnthropicResponse {
            content: vec![ContentBlock::Text {
                text: "I inspected the top-level layout and still need a structured follow-up."
                    .to_string(),
            }],
            stop_reason: Some("end_turn".to_string()),
            usage: Some(TokenUsage::default()),
        },
        AnthropicResponse {
            content: vec![ContentBlock::Text {
                text: "<request_user_input>\nquestion: Which area should the plan focus on first?\noption: Prompt runtime | Tighten the planning prompt and continuation phases.\noption: TUI lifecycle | Improve the planning and approval interaction flow.\n</request_user_input>".to_string(),
            }],
            stop_reason: Some("end_turn".to_string()),
            usage: Some(TokenUsage::default()),
        },
    ]));

    let mut tool_manager = ToolManager::new();
    tool_manager.register(Box::new(StubTool));
    let mut agent = Agent::new(
        tool_manager,
        backend,
        Arc::new(VectorDB::new("data/lancedb")),
        session_manager.clone(),
        workspace.clone(),
    );
    agent.tool_result_store =
        ToolResultStore::new(rara_dir.join("tool-results")).expect("tool result store");
    agent.set_execution_mode(AgentExecutionMode::Plan);

    agent
        .query_with_mode("inspect".to_string(), super::AgentOutputMode::Silent)
        .await
        .expect("query should succeed");

    assert!(!agent.last_query_produced_plan());

    let backend = Arc::new(SequencedBackend::new(vec![AnthropicResponse {
        content: vec![ContentBlock::Text {
            text: "<plan>\n- [pending] Inspect the repository structure\n- [pending] Review src/main.rs bootstrap flow\n</plan>\nReady for approval.".to_string(),
        }],
        stop_reason: Some("end_turn".to_string()),
        usage: Some(TokenUsage::default()),
    }]));

    let mut tool_manager = ToolManager::new();
    tool_manager.register(Box::new(StubTool));
    let mut agent = Agent::new(
        tool_manager,
        backend,
        Arc::new(VectorDB::new("data/lancedb")),
        session_manager,
        workspace,
    );
    agent.tool_result_store =
        ToolResultStore::new(rara_dir.join("tool-results")).expect("tool result store");
    agent.set_execution_mode(AgentExecutionMode::Plan);

    agent
        .query_with_mode("inspect".to_string(), super::AgentOutputMode::Silent)
        .await
        .expect("query should succeed");

    assert!(agent.last_query_produced_plan());
}

#[tokio::test]
async fn continues_plan_mode_after_exploration_if_assistant_still_signals_more_work() {
    let backend = Arc::new(SequencedBackend::new(vec![
        AnthropicResponse {
            content: vec![
                ContentBlock::Text {
                    text: "<plan>\n- [pending] Inspect the repository structure\n</plan>\nStart with the top-level layout.".to_string(),
                },
                ContentBlock::ToolUse {
                    id: "tool-1".to_string(),
                    name: "stub_tool".to_string(),
                    input: json!({}),
                },
            ],
            stop_reason: Some("tool_use".to_string()),
            usage: Some(TokenUsage::default()),
        },
        AnthropicResponse {
            content: vec![ContentBlock::Text {
                text: "I have examined the overall structure and still need more inspection.\n<continue_inspection/>".to_string(),
            }],
            stop_reason: Some("end_turn".to_string()),
            usage: Some(TokenUsage::default()),
        },
        AnthropicResponse {
            content: vec![ContentBlock::Text {
                text: "<plan>\n- [pending] Inspect src/main.rs bootstrap flow\n- [pending] Review the prompt runtime wiring\n</plan>\nThe inspection path is now complete."
                    .to_string(),
            }],
            stop_reason: Some("end_turn".to_string()),
            usage: Some(TokenUsage::default()),
        },
    ]));

    let mut tool_manager = ToolManager::new();
    tool_manager.register(Box::new(StubTool));
    let mut agent = Agent::new(
        tool_manager,
        backend.clone(),
        Arc::new(VectorDB::new("data/lancedb")),
        Arc::new(SessionManager::new().expect("session manager")),
        Arc::new(WorkspaceMemory::new().expect("workspace memory")),
    );
    agent.set_execution_mode(AgentExecutionMode::Plan);

    agent
        .query_with_mode("inspect".to_string(), super::AgentOutputMode::Silent)
        .await
        .expect("query should succeed");

    let observed = backend.observed_messages.lock().expect("lock");
    assert_eq!(observed.len(), 3);
    assert!(agent.history.iter().any(|message| message
        .content
        .to_string()
        .contains("plan_continuation_required")));
}

#[tokio::test]
async fn continues_plan_mode_to_synthesize_plan_after_exploration_evidence() {
    let backend = Arc::new(SequencedBackend::new(vec![
        AnthropicResponse {
            content: vec![ContentBlock::ToolUse {
                id: "tool-1".to_string(),
                name: "list_files".to_string(),
                input: json!({}),
            }],
            stop_reason: Some("tool_use".to_string()),
            usage: Some(TokenUsage::default()),
        },
        AnthropicResponse {
            content: vec![ContentBlock::Text {
                text: "I inspected the repository structure and the current planning flow."
                    .to_string(),
            }],
            stop_reason: Some("end_turn".to_string()),
            usage: Some(TokenUsage::default()),
        },
        AnthropicResponse {
            content: vec![ContentBlock::Text {
                text: "<plan>\n- [pending] Review planning continuation state\n- [pending] Tighten plan completion rules\n</plan>\nThe first pass exposed enough evidence to finalize the plan.".to_string(),
            }],
            stop_reason: Some("end_turn".to_string()),
            usage: Some(TokenUsage::default()),
        },
    ]));

    let mut tool_manager = ToolManager::new();
    tool_manager.register(Box::new(ListFilesStub));
    let mut agent = Agent::new(
        tool_manager,
        backend.clone(),
        Arc::new(VectorDB::new("data/lancedb")),
        Arc::new(SessionManager::new().expect("session manager")),
        Arc::new(WorkspaceMemory::new().expect("workspace memory")),
    );
    agent.set_execution_mode(AgentExecutionMode::Plan);

    agent
        .query_with_mode("inspect".to_string(), super::AgentOutputMode::Silent)
        .await
        .expect("query should succeed");

    let observed = backend.observed_messages.lock().expect("lock");
    assert_eq!(observed.len(), 3);
    assert!(agent.history.iter().any(|message| message
        .content
        .to_string()
        .contains("plan_structured_outcome_required")));
    assert_eq!(agent.current_plan.len(), 2);
}

#[tokio::test]
async fn narration_only_planning_turn_requires_structured_followup() {
    let backend = Arc::new(SequencedBackend::new(vec![
        AnthropicResponse {
            content: vec![ContentBlock::Text {
                text: "I reviewed the prompt runtime and the workspace discovery flow."
                    .to_string(),
            }],
            stop_reason: Some("end_turn".to_string()),
            usage: Some(TokenUsage::default()),
        },
        AnthropicResponse {
            content: vec![ContentBlock::Text {
                text: "<plan>\n- [pending] Generalize instruction discovery\n- [pending] Preserve current cache behavior\n</plan>\nThe inspected code now supports a concrete refactor path.".to_string(),
            }],
            stop_reason: Some("end_turn".to_string()),
            usage: Some(TokenUsage::default()),
        },
    ]));

    let (_temp, session_manager, workspace, rara_dir) = test_runtime_storage();
    let mut agent = Agent::new(
        ToolManager::new(),
        backend.clone(),
        Arc::new(VectorDB::new("data/lancedb")),
        session_manager,
        workspace,
    );
    agent.tool_result_store =
        ToolResultStore::new(rara_dir.join("tool-results")).expect("tool result store");
    agent.set_execution_mode(AgentExecutionMode::Plan);

    agent
        .query_with_mode("inspect".to_string(), super::AgentOutputMode::Silent)
        .await
        .expect("query should succeed");

    let observed = backend.observed_messages.lock().expect("lock");
    assert_eq!(observed.len(), 2);
    assert!(agent.history.iter().any(|message| {
        message
            .content
            .to_string()
            .contains("plan_structured_outcome_required")
    }));
    assert_eq!(agent.current_plan.len(), 2);
}

#[tokio::test]
async fn delegated_plan_agent_counts_as_planning_evidence() {
    let backend = Arc::new(SequencedBackend::new(vec![
        AnthropicResponse {
            content: vec![ContentBlock::ToolUse {
                id: "tool-1".to_string(),
                name: "plan_agent".to_string(),
                input: json!({ "task": "inspect planning flow" }),
            }],
            stop_reason: Some("tool_use".to_string()),
            usage: Some(TokenUsage::default()),
        },
        AnthropicResponse {
            content: vec![ContentBlock::Text {
                text: "The delegated planning pass inspected enough code context.".to_string(),
            }],
            stop_reason: Some("end_turn".to_string()),
            usage: Some(TokenUsage::default()),
        },
        AnthropicResponse {
            content: vec![ContentBlock::Text {
                text: "<plan>\n- [pending] Review delegated planning evidence\n- [pending] Finalize the top-level plan\n</plan>\nReady for approval.".to_string(),
            }],
            stop_reason: Some("end_turn".to_string()),
            usage: Some(TokenUsage::default()),
        },
    ]));

    let mut tool_manager = ToolManager::new();
    tool_manager.register(Box::new(PlanAgentStub));
    let mut agent = Agent::new(
        tool_manager,
        backend.clone(),
        Arc::new(VectorDB::new("data/lancedb")),
        Arc::new(SessionManager::new().expect("session manager")),
        Arc::new(WorkspaceMemory::new().expect("workspace memory")),
    );
    agent.set_execution_mode(AgentExecutionMode::Plan);

    agent
        .query_with_mode("inspect".to_string(), super::AgentOutputMode::Silent)
        .await
        .expect("query should succeed");

    let observed = backend.observed_messages.lock().expect("lock");
    assert_eq!(observed.len(), 3);
    assert_eq!(agent.current_plan.len(), 2);
}

#[tokio::test]
async fn continues_execute_mode_after_exploration_if_assistant_still_signals_more_work() {
    let backend = Arc::new(SequencedBackend::new(vec![
        AnthropicResponse {
            content: vec![ContentBlock::ToolUse {
                id: "tool-1".to_string(),
                name: "stub_tool".to_string(),
                input: json!({}),
            }],
            stop_reason: Some("tool_use".to_string()),
            usage: Some(TokenUsage::default()),
        },
        AnthropicResponse {
            content: vec![ContentBlock::Text {
                text: "I have checked the top-level structure and need one more inspection pass.\n<continue_inspection/>".to_string(),
            }],
            stop_reason: Some("end_turn".to_string()),
            usage: Some(TokenUsage::default()),
        },
        AnthropicResponse {
            content: vec![ContentBlock::Text {
                text: "done".to_string(),
            }],
            stop_reason: Some("end_turn".to_string()),
            usage: Some(TokenUsage::default()),
        },
    ]));

    let mut tool_manager = ToolManager::new();
    tool_manager.register(Box::new(StubTool));
    let mut agent = Agent::new(
        tool_manager,
        backend.clone(),
        Arc::new(VectorDB::new("data/lancedb")),
        Arc::new(SessionManager::new().expect("session manager")),
        Arc::new(WorkspaceMemory::new().expect("workspace memory")),
    );
    agent.set_execution_mode(AgentExecutionMode::Execute);

    agent
        .query_with_mode("inspect".to_string(), super::AgentOutputMode::Silent)
        .await
        .expect("query should succeed");

    let observed = backend.observed_messages.lock().expect("lock");
    assert_eq!(observed.len(), 3);
    assert!(agent.history.iter().any(|message| message
        .content
        .to_string()
        .contains("execution_continuation_required")));
}

#[tokio::test]
async fn continues_execute_mode_when_assistant_requests_structured_followup_inspection() {
    let backend = Arc::new(SequencedBackend::new(vec![
        AnthropicResponse {
            content: vec![ContentBlock::Text {
                text: "I have checked the repository layout and need one more repo-inspection step.\n<continue_inspection/>".to_string(),
            }],
            stop_reason: Some("end_turn".to_string()),
            usage: Some(TokenUsage::default()),
        },
        AnthropicResponse {
            content: vec![ContentBlock::Text {
                text: "done".to_string(),
            }],
            stop_reason: Some("end_turn".to_string()),
            usage: Some(TokenUsage::default()),
        },
    ]));

    let mut agent = Agent::new(
        ToolManager::new(),
        backend.clone(),
        Arc::new(VectorDB::new("data/lancedb")),
        Arc::new(SessionManager::new().expect("session manager")),
        Arc::new(WorkspaceMemory::new().expect("workspace memory")),
    );
    agent.set_execution_mode(AgentExecutionMode::Execute);

    agent
        .query_with_mode("inspect".to_string(), super::AgentOutputMode::Silent)
        .await
        .expect("query should succeed");

    let observed = backend.observed_messages.lock().expect("lock");
    assert_eq!(observed.len(), 2);
    assert!(agent.history.iter().any(|message| message
        .content
        .to_string()
        .contains("execution_continuation_required")));
}

#[test]
fn strips_continue_inspection_control_tag() {
    let (cleaned, requested) =
        strip_continue_inspection_control("Need one more inspection pass.\n<continue_inspection/>");
    assert!(requested);
    assert_eq!(cleaned, "Need one more inspection pass.\n");

    let (cleaned, requested) = strip_continue_inspection_control("Final answer");
    assert!(!requested);
    assert_eq!(cleaned, "Final answer");
}

#[test]
fn parses_structured_plan_block() {
    let text = "<plan>\n- [in_progress] Inspect core agent loop\n- [pending] Review TUI rendering path\n- [completed] Confirm current constraints\n</plan>\nFocus on agent.rs and tui/runtime.rs first.";
    let parsed = parse_plan_block(text).expect("plan block should parse");
    assert_eq!(
        parsed.0,
        vec![
            PlanStep {
                step: "Inspect core agent loop".to_string(),
                status: PlanStepStatus::InProgress,
            },
            PlanStep {
                step: "Review TUI rendering path".to_string(),
                status: PlanStepStatus::Pending,
            },
            PlanStep {
                step: "Confirm current constraints".to_string(),
                status: PlanStepStatus::Completed,
            },
        ]
    );
    assert_eq!(
        parsed.1.as_deref(),
        Some("Focus on agent.rs and tui/runtime.rs first.")
    );
}

#[test]
fn parses_request_user_input_block() {
    let text = "<request_user_input>\nquestion: Which path should we take first?\noption: Minimal | Keep the diff small and local.\noption: Broad | Reshape the module boundaries now.\n</request_user_input>\nNeed direction before editing.";
    let parsed = parse_request_user_input_block(text).expect("question block should parse");
    assert_eq!(
        parsed,
        PendingUserInput {
            question: "Which path should we take first?".to_string(),
            options: vec![
                (
                    "Minimal".to_string(),
                    "Keep the diff small and local.".to_string(),
                ),
                (
                    "Broad".to_string(),
                    "Reshape the module boundaries now.".to_string(),
                ),
            ],
            note: Some("Need direction before editing.".to_string()),
        }
    );
}

#[test]
fn advances_plan_steps_during_execute_mode() {
    let mut agent = Agent::new(
        ToolManager::new(),
        Arc::new(SequencedBackend::new(Vec::new())),
        Arc::new(VectorDB::new("data/lancedb")),
        Arc::new(SessionManager::new().expect("session manager")),
        Arc::new(WorkspaceMemory::new().expect("workspace memory")),
    );
    agent.set_execution_mode(AgentExecutionMode::Execute);
    agent.current_plan = vec![
        PlanStep {
            step: "Inspect code".to_string(),
            status: PlanStepStatus::Pending,
        },
        PlanStep {
            step: "Apply changes".to_string(),
            status: PlanStepStatus::Pending,
        },
    ];

    agent.ensure_active_plan_step();
    assert_eq!(agent.current_plan[0].status, PlanStepStatus::InProgress);
    assert_eq!(agent.current_plan[1].status, PlanStepStatus::Pending);

    agent.advance_plan_step();
    assert_eq!(agent.current_plan[0].status, PlanStepStatus::Completed);
    assert_eq!(agent.current_plan[1].status, PlanStepStatus::InProgress);
}

#[test]
fn completes_only_active_plan_step_on_finish() {
    let mut agent = Agent::new(
        ToolManager::new(),
        Arc::new(SequencedBackend::new(Vec::new())),
        Arc::new(VectorDB::new("data/lancedb")),
        Arc::new(SessionManager::new().expect("session manager")),
        Arc::new(WorkspaceMemory::new().expect("workspace memory")),
    );
    agent.set_execution_mode(AgentExecutionMode::Execute);
    agent.current_plan = vec![
        PlanStep {
            step: "Inspect code".to_string(),
            status: PlanStepStatus::Completed,
        },
        PlanStep {
            step: "Apply changes".to_string(),
            status: PlanStepStatus::InProgress,
        },
        PlanStep {
            step: "Summarize".to_string(),
            status: PlanStepStatus::Pending,
        },
    ];

    agent.complete_active_plan_step();

    assert_eq!(agent.current_plan[0].status, PlanStepStatus::Completed);
    assert_eq!(agent.current_plan[1].status, PlanStepStatus::Completed);
    assert_eq!(agent.current_plan[2].status, PlanStepStatus::Pending);
}

#[tokio::test]
async fn manual_compact_replaces_older_history_with_summary() {
    let backend = Arc::new(SequencedBackend::new(Vec::new()));
    let mut agent = Agent::new(
        ToolManager::new(),
        backend,
        Arc::new(VectorDB::new("data/lancedb")),
        Arc::new(SessionManager::new().expect("session manager")),
        Arc::new(WorkspaceMemory::new().expect("workspace memory")),
    );
    agent.history = vec![
        Message {
            role: "user".to_string(),
            content: json!("inspect the repo"),
        },
        Message {
            role: "assistant".to_string(),
            content: json!("I checked Cargo.toml and src/main.rs"),
        },
    ];

    let compacted = agent
        .compact_now_with_reporter(|_| {})
        .await
        .expect("compact should succeed");

    assert!(compacted);
    assert_eq!(agent.compact_state.compaction_count, 1);
    assert_eq!(agent.history[0].role, "system");
    let boundary = agent.history[0]
        .content
        .as_object()
        .expect("compact boundary");
    assert_eq!(
        boundary.get("type").and_then(serde_json::Value::as_str),
        Some("compact_boundary")
    );
    assert!(agent.history[1]
        .content
        .to_string()
        .contains("STRUCTURED SUMMARY OF PREVIOUS CONVERSATION"));
}

#[tokio::test]
async fn manual_compact_carries_recent_files_forward() {
    let backend = Arc::new(SequencedBackend::new(Vec::new()));
    let mut agent = Agent::new(
        ToolManager::new(),
        backend,
        Arc::new(VectorDB::new("data/lancedb")),
        Arc::new(SessionManager::new().expect("session manager")),
        Arc::new(WorkspaceMemory::new().expect("workspace memory")),
    );
    agent.history = vec![
        Message {
            role: "assistant".to_string(),
            content: json!([
                {"type":"tool_use","id":"tool-1","name":"read_file","input":{"path":"src/main.rs"}},
                {"type":"tool_use","id":"tool-2","name":"list_files","input":{"path":"src/agent"}}
            ]),
        },
        Message {
            role: "user".to_string(),
            content: json!([
                {"type":"tool_result","tool_use_id":"tool-1","content":"fn main() {}"},
                {"type":"tool_result","tool_use_id":"tool-2","content":"planning.rs"}
            ]),
        },
        Message {
            role: "assistant".to_string(),
            content: json!("I inspected the relevant files."),
        },
    ];

    let compacted = agent
        .compact_now_with_reporter(|_| {})
        .await
        .expect("compact should succeed");

    assert!(compacted);
    assert_eq!(agent.history[2].role, "system");
    assert_eq!(agent.history[3].role, "system");
    let boundary = agent.history[0]
        .content
        .as_object()
        .expect("compact boundary");
    assert_eq!(
        boundary
            .get("recent_file_count")
            .and_then(serde_json::Value::as_u64),
        Some(2)
    );
    let recent_files = agent.history[2].content.to_string();
    assert!(recent_files.contains("RECENT FILES FROM COMPACTED HISTORY"));
    assert!(recent_files.contains("src/main.rs"));
    assert!(recent_files.contains("src/agent"));
    let excerpts = agent.history[3].content.to_string();
    assert!(excerpts.contains("RECENT FILE EXCERPTS FROM COMPACTED HISTORY"));
    assert!(excerpts.contains("### src/main.rs"));
    assert!(excerpts.contains("fn main() {}"));
}

#[tokio::test]
async fn manual_compact_prefers_latest_excerpt_and_tracks_apply_patch() {
    let backend = Arc::new(SequencedBackend::new(Vec::new()));
    let mut agent = Agent::new(
        ToolManager::new(),
        backend,
        Arc::new(VectorDB::new("data/lancedb")),
        Arc::new(SessionManager::new().expect("session manager")),
        Arc::new(WorkspaceMemory::new().expect("workspace memory")),
    );
    agent.history = vec![
        Message {
            role: "assistant".to_string(),
            content: json!([
                {"type":"tool_use","id":"tool-1","name":"read_file","input":{"path":"src/main.rs","start_line":1,"end_line":2}},
                {"type":"tool_use","id":"tool-2","name":"apply_patch","input":{"path":"src/lib.rs"}}
            ]),
        },
        Message {
            role: "user".to_string(),
            content: json!([
                {"type":"tool_result","tool_use_id":"tool-1","content":"old snippet"},
                {"type":"tool_result","tool_use_id":"tool-2","content":"patch applied"}
            ]),
        },
        Message {
            role: "assistant".to_string(),
            content: json!([
                {"type":"tool_use","id":"tool-3","name":"read_file","input":{"path":"src/main.rs","start_line":10,"end_line":12}}
            ]),
        },
        Message {
            role: "user".to_string(),
            content: json!([
                {"type":"tool_result","tool_use_id":"tool-3","content":"new snippet"}
            ]),
        },
        Message {
            role: "assistant".to_string(),
            content: json!("I updated the inspection notes."),
        },
    ];

    let compacted = agent
        .compact_now_with_reporter(|_| {})
        .await
        .expect("compact should succeed");

    assert!(compacted);
    let recent_files = agent.history[2].content.to_string();
    assert!(recent_files.contains("src/lib.rs"));
    let excerpts = agent.history[3].content.to_string();
    assert!(excerpts.contains("new snippet"));
    assert!(!excerpts.contains("old snippet"));
    assert!(excerpts.contains("lines 10-12"));
}
