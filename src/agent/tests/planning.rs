use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::json;

use crate::agent::planning::{
    parse_plan_block, parse_request_user_input_block, strip_continue_inspection_control,
};
use crate::agent::{
    Agent, AgentExecutionMode, BashApprovalDecision, ContentBlock, PendingUserInput, PlanStep,
    PlanStepStatus, RuntimeContinuationPhase,
};
use crate::llm::{LlmBackend, LlmResponse, TokenUsage};
use crate::session::SessionManager;
use crate::tool::ToolManager;
use crate::tool_result::ToolResultStore;
use crate::tools::planning::EnterPlanModeTool;
use crate::vectordb::VectorDB;
use crate::workspace::WorkspaceMemory;

use super::support::{test_runtime_storage, SequencedBackend, StubBashTool, StubTool};

struct CheckpointObserverBackend {
    session_manager: Arc<SessionManager>,
    session_id: String,
}

#[async_trait]
impl LlmBackend for CheckpointObserverBackend {
    async fn ask(
        &self,
        _messages: &[crate::agent::Message],
        _tools: &[serde_json::Value],
    ) -> Result<LlmResponse> {
        let persisted = self
            .session_manager
            .load_thread_history(&self.session_id)
            .expect("user message should be checkpointed before model call");
        assert!(persisted.iter().any(|message| {
            message.role == "user" && message.content.to_string().contains("checkpoint me")
        }));
        Ok(LlmResponse {
            content: vec![ContentBlock::Text {
                text: "checkpoint observed".to_string(),
            }],
            stop_reason: Some("end_turn".to_string()),
            usage: Some(TokenUsage::default()),
        })
    }

    async fn embed(&self, _text: &str) -> Result<Vec<f32>> {
        Ok(vec![0.0; 8])
    }

    async fn summarize(
        &self,
        _messages: &[crate::agent::Message],
        _instruction: &str,
    ) -> Result<String> {
        Ok("summary".to_string())
    }
}

#[tokio::test]
async fn appends_continuation_after_tool_result() {
    let backend = Arc::new(SequencedBackend::new(vec![
        LlmResponse {
            content: vec![ContentBlock::ToolUse {
                id: "tool-1".to_string(),
                name: "stub_tool".to_string(),
                input: json!({}),
            }],
            stop_reason: Some("tool_use".to_string()),
            usage: Some(TokenUsage::default()),
        },
        LlmResponse {
            content: vec![ContentBlock::Text {
                text: "done".to_string(),
            }],
            stop_reason: Some("end_turn".to_string()),
            usage: Some(TokenUsage::default()),
        },
    ]));

    let mut tool_manager = ToolManager::new();
    tool_manager.register(Box::new(StubTool));
    let (_temp, session_manager, workspace, rara_dir) = test_runtime_storage();
    let mut agent = Agent::new(
        tool_manager,
        backend.clone(),
        Arc::new(VectorDB::new(&rara_dir.join("lancedb").to_string_lossy())),
        session_manager,
        workspace,
    );

    agent
        .query_with_mode("do work".to_string(), super::super::AgentOutputMode::Silent)
        .await
        .expect("query should succeed");

    let observed = backend.observed_messages();
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
async fn suggestion_mode_auto_allows_read_only_bash_commands() {
    let backend = Arc::new(SequencedBackend::new(vec![
        LlmResponse {
            content: vec![ContentBlock::ToolUse {
                id: "tool-readonly-bash".to_string(),
                name: "bash".to_string(),
                input: json!({ "command": "git status --short" }),
            }],
            stop_reason: Some("tool_use".to_string()),
            usage: Some(TokenUsage::default()),
        },
        LlmResponse {
            content: vec![ContentBlock::Text {
                text: "done".to_string(),
            }],
            stop_reason: Some("end_turn".to_string()),
            usage: Some(TokenUsage::default()),
        },
    ]));
    let mut tool_manager = ToolManager::new();
    tool_manager.register(Box::new(StubBashTool));
    let (_temp, session_manager, workspace, rara_dir) = test_runtime_storage();
    let mut agent = Agent::new(
        tool_manager,
        backend.clone(),
        Arc::new(VectorDB::new(&rara_dir.join("lancedb").to_string_lossy())),
        session_manager,
        workspace,
    );
    agent.bash_approval_mode = crate::agent::BashApprovalMode::Suggestion;

    agent
        .query_with_mode(
            "inspect git state".to_string(),
            super::super::AgentOutputMode::Silent,
        )
        .await
        .expect("query should auto-allow read-only bash");

    assert!(agent.pending_approval.is_none());
    assert_eq!(backend.observed_messages().len(), 2);
}

#[tokio::test]
async fn suggestion_mode_keeps_write_bash_commands_pending_approval() {
    let backend = Arc::new(SequencedBackend::new(vec![LlmResponse {
        content: vec![ContentBlock::ToolUse {
            id: "tool-write-bash".to_string(),
            name: "bash".to_string(),
            input: json!({ "command": "git push origin main" }),
        }],
        stop_reason: Some("tool_use".to_string()),
        usage: Some(TokenUsage::default()),
    }]));
    let mut tool_manager = ToolManager::new();
    tool_manager.register(Box::new(StubBashTool));
    let (_temp, session_manager, workspace, rara_dir) = test_runtime_storage();
    let mut agent = Agent::new(
        tool_manager,
        backend.clone(),
        Arc::new(VectorDB::new(&rara_dir.join("lancedb").to_string_lossy())),
        session_manager,
        workspace,
    );
    agent.bash_approval_mode = crate::agent::BashApprovalMode::Suggestion;

    agent
        .query_with_mode(
            "push changes".to_string(),
            super::super::AgentOutputMode::Silent,
        )
        .await
        .expect("query should pause on write bash approval");

    assert!(agent.pending_approval.is_some());
    assert_eq!(backend.observed_messages().len(), 1);
}

#[tokio::test]
async fn plan_mode_allows_read_only_bash_commands() {
    let backend = Arc::new(SequencedBackend::new(vec![
        LlmResponse {
            content: vec![ContentBlock::ToolUse {
                id: "tool-readonly-bash-plan".to_string(),
                name: "bash".to_string(),
                input: json!({ "command": "git status --short" }),
            }],
            stop_reason: Some("tool_use".to_string()),
            usage: Some(TokenUsage::default()),
        },
        LlmResponse {
            content: vec![ContentBlock::Text {
                text: "Read-only inspection complete.".to_string(),
            }],
            stop_reason: Some("end_turn".to_string()),
            usage: Some(TokenUsage::default()),
        },
    ]));
    let mut tool_manager = ToolManager::new();
    tool_manager.register(Box::new(StubBashTool));
    let (_temp, session_manager, workspace, rara_dir) = test_runtime_storage();
    let mut agent = Agent::new(
        tool_manager,
        backend.clone(),
        Arc::new(VectorDB::new(&rara_dir.join("lancedb").to_string_lossy())),
        session_manager,
        workspace,
    );
    agent.set_execution_mode(AgentExecutionMode::Plan);

    agent
        .query_with_mode(
            "inspect git state".to_string(),
            super::super::AgentOutputMode::Silent,
        )
        .await
        .expect("query should allow read-only bash in plan mode");

    assert_eq!(agent.execution_mode, AgentExecutionMode::Plan);
    assert!(agent.pending_approval.is_none());
    assert_eq!(backend.observed_messages().len(), 2);
}

#[tokio::test]
async fn plan_mode_rejects_mutating_bash_commands_without_approval() {
    let backend = Arc::new(SequencedBackend::new(vec![
        LlmResponse {
            content: vec![ContentBlock::ToolUse {
                id: "tool-write-bash-plan".to_string(),
                name: "bash".to_string(),
                input: json!({ "command": "git push origin main" }),
            }],
            stop_reason: Some("tool_use".to_string()),
            usage: Some(TokenUsage::default()),
        },
        LlmResponse {
            content: vec![ContentBlock::Text {
                text: "I will return a plan instead.".to_string(),
            }],
            stop_reason: Some("end_turn".to_string()),
            usage: Some(TokenUsage::default()),
        },
    ]));
    let mut tool_manager = ToolManager::new();
    tool_manager.register(Box::new(StubBashTool));
    let (_temp, session_manager, workspace, rara_dir) = test_runtime_storage();
    let mut agent = Agent::new(
        tool_manager,
        backend.clone(),
        Arc::new(VectorDB::new(&rara_dir.join("lancedb").to_string_lossy())),
        session_manager,
        workspace,
    );
    agent.set_execution_mode(AgentExecutionMode::Plan);

    agent
        .query_with_mode(
            "push changes".to_string(),
            super::super::AgentOutputMode::Silent,
        )
        .await
        .expect("query should reject mutating bash and continue");

    assert_eq!(agent.execution_mode, AgentExecutionMode::Plan);
    assert!(agent.pending_approval.is_none());
    assert_eq!(backend.observed_messages().len(), 2);
    assert!(agent.history.iter().any(|message| message
        .content
        .to_string()
        .contains("bash is read-only in plan mode")));
}

#[tokio::test]
async fn approved_bash_prefix_auto_allows_later_matching_commands() {
    let backend = Arc::new(SequencedBackend::new(vec![
        LlmResponse {
            content: vec![ContentBlock::ToolUse {
                id: "tool-first-push".to_string(),
                name: "bash".to_string(),
                input: json!({ "command": "git push origin main" }),
            }],
            stop_reason: Some("tool_use".to_string()),
            usage: Some(TokenUsage::default()),
        },
        LlmResponse {
            content: vec![ContentBlock::Text {
                text: "first push done".to_string(),
            }],
            stop_reason: Some("end_turn".to_string()),
            usage: Some(TokenUsage::default()),
        },
        LlmResponse {
            content: vec![ContentBlock::ToolUse {
                id: "tool-second-push".to_string(),
                name: "bash".to_string(),
                input: json!({ "command": "git push origin feature" }),
            }],
            stop_reason: Some("tool_use".to_string()),
            usage: Some(TokenUsage::default()),
        },
        LlmResponse {
            content: vec![ContentBlock::Text {
                text: "second push done".to_string(),
            }],
            stop_reason: Some("end_turn".to_string()),
            usage: Some(TokenUsage::default()),
        },
    ]));
    let mut tool_manager = ToolManager::new();
    tool_manager.register(Box::new(StubBashTool));
    let (_temp, session_manager, workspace, rara_dir) = test_runtime_storage();
    let mut agent = Agent::new(
        tool_manager,
        backend.clone(),
        Arc::new(VectorDB::new(&rara_dir.join("lancedb").to_string_lossy())),
        session_manager,
        workspace,
    );
    agent.bash_approval_mode = crate::agent::BashApprovalMode::Suggestion;

    agent
        .query_with_mode(
            "push once".to_string(),
            super::super::AgentOutputMode::Silent,
        )
        .await
        .expect("first query should pause on approval");
    assert!(agent.pending_approval.is_some());

    agent
        .answer_pending_approval_with_events(
            BashApprovalDecision::Prefix,
            super::super::AgentOutputMode::Silent,
            |_| {},
        )
        .await
        .expect("prefix approval should execute pending command");
    assert_eq!(agent.approved_bash_prefixes, vec!["git push".to_string()]);

    agent
        .query_with_mode(
            "push matching prefix".to_string(),
            super::super::AgentOutputMode::Silent,
        )
        .await
        .expect("matching prefix should auto-allow bash");

    assert!(agent.pending_approval.is_none());
    assert_eq!(backend.observed_messages().len(), 4);
}

#[tokio::test]
async fn checkpoints_user_message_before_first_model_turn() {
    let (_temp, session_manager, workspace, rara_dir) = test_runtime_storage();
    let session_id = "checkpoint-before-model".to_string();
    let backend = Arc::new(CheckpointObserverBackend {
        session_manager: session_manager.clone(),
        session_id: session_id.clone(),
    });
    let mut agent = Agent::new(
        ToolManager::new(),
        backend,
        Arc::new(VectorDB::new(
            &rara_dir.join("lancedb").display().to_string(),
        )),
        session_manager,
        workspace,
    );
    agent.session_id = session_id;
    agent.tool_result_store =
        ToolResultStore::new(rara_dir.join("tool-results")).expect("tool result store");

    agent
        .query_with_mode(
            "checkpoint me".to_string(),
            super::super::AgentOutputMode::Silent,
        )
        .await
        .expect("query should succeed");
}

#[tokio::test]
async fn resumes_after_plan_approval_via_structured_continuation() {
    let (_temp, session_manager, workspace, rara_dir) = test_runtime_storage();
    let backend = Arc::new(SequencedBackend::new(vec![LlmResponse {
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
        .resume_after_plan_approval_with_events(
            false,
            super::super::AgentOutputMode::Silent,
            |_| {},
        )
        .await
        .expect("resume should succeed");

    let observed = backend.observed_messages();
    assert_eq!(observed.len(), 1);
    let runtime_texts = observed[0]
        .iter()
        .filter_map(|message| message.content.as_array())
        .flat_map(|blocks| blocks.iter())
        .filter_map(|block| block.get("text").and_then(serde_json::Value::as_str))
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
    let backend = Arc::new(SequencedBackend::new(vec![LlmResponse {
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
        .query_with_mode("hello".to_string(), super::super::AgentOutputMode::Silent)
        .await
        .expect("query should succeed");

    assert_eq!(backend.observed_messages().len(), 1);
    assert!(!agent.history.iter().any(|message| message
        .content
        .to_string()
        .contains("\"phase\": \"tool_results_available\"")));
}

#[tokio::test]
async fn enter_plan_mode_tool_switches_to_read_only_planning() {
    let backend = Arc::new(SequencedBackend::new(vec![
        LlmResponse {
            content: vec![ContentBlock::ToolUse {
                id: "enter-plan".to_string(),
                name: "enter_plan_mode".to_string(),
                input: json!({}),
            }],
            stop_reason: Some("tool_use".to_string()),
            usage: Some(TokenUsage::default()),
        },
        LlmResponse {
            content: vec![ContentBlock::Text {
                text: "The main issue is that planning and approval are coupled.".to_string(),
            }],
            stop_reason: Some("end_turn".to_string()),
            usage: Some(TokenUsage::default()),
        },
    ]));

    let mut tool_manager = ToolManager::new();
    tool_manager.register(Box::new(EnterPlanModeTool));
    let (_temp, session_manager, workspace, rara_dir) = test_runtime_storage();
    let mut agent = Agent::new(
        tool_manager,
        backend.clone(),
        Arc::new(VectorDB::new(&rara_dir.join("lancedb").to_string_lossy())),
        session_manager,
        workspace,
    );

    agent
        .query_with_mode(
            "review the planning implementation".to_string(),
            super::super::AgentOutputMode::Silent,
        )
        .await
        .expect("query should enter planning mode and return analysis");

    assert_eq!(agent.execution_mode, AgentExecutionMode::Plan);
    assert!(!agent.last_query_produced_plan());
    assert!(agent.current_plan.is_empty());

    let observed_tools = backend.observed_tools();
    assert_eq!(observed_tools.len(), 2);
    assert!(observed_tools[0].contains(&"enter_plan_mode".to_string()));
    assert!(!observed_tools[1].contains(&"enter_plan_mode".to_string()));
}

#[tokio::test]
async fn continues_tool_loop_without_fixed_turn_cap() {
    let tool_turns = 205;
    let mut responses = (0..tool_turns)
        .map(|idx| LlmResponse {
            content: vec![ContentBlock::ToolUse {
                id: format!("tool-{idx}"),
                name: "stub_tool".to_string(),
                input: json!({}),
            }],
            stop_reason: Some("tool_use".to_string()),
            usage: Some(TokenUsage::default()),
        })
        .collect::<Vec<_>>();
    responses.push(LlmResponse {
        content: vec![ContentBlock::Text {
            text: "Final answer after reviewing the tool results.".to_string(),
        }],
        stop_reason: Some("end_turn".to_string()),
        usage: Some(TokenUsage::default()),
    });
    let backend = Arc::new(SequencedBackend::new(responses));

    let mut tool_manager = ToolManager::new();
    tool_manager.register(Box::new(StubTool));
    let (_temp, session_manager, workspace, rara_dir) = test_runtime_storage();
    let mut agent = Agent::new(
        tool_manager,
        backend.clone(),
        Arc::new(VectorDB::new(&rara_dir.join("lancedb").to_string_lossy())),
        session_manager,
        workspace,
    );

    agent
        .query_with_mode("loop".to_string(), super::super::AgentOutputMode::Silent)
        .await
        .expect("query should continue until the model returns a final answer");

    let observed_tools = backend.observed_tools();
    assert_eq!(
        observed_tools.len(),
        tool_turns + 1,
        "the agent should continue past the former fixed turn cap before the final answer"
    );
    assert!(agent.history.last().is_some_and(|message| message
        .content
        .to_string()
        .contains("Final answer after reviewing the tool results.")));
    assert!(agent
        .history
        .iter()
        .any(|message| message.content.to_string().contains("tool-204")));
    assert_no_unresolved_tool_uses(&agent.history);
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
    let text = "<proposed_plan>\n- [in_progress] Inspect core agent loop\n- Review TUI rendering path\n1. Confirm current constraints\n</proposed_plan>\nFocus on agent.rs and tui/runtime.rs first.";
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
                status: PlanStepStatus::Pending,
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

fn new_planning_agent() -> Agent {
    let (_temp, session_manager, workspace, rara_dir) = test_runtime_storage();
    let mut agent = Agent::new(
        ToolManager::new(),
        Arc::new(SequencedBackend::new(Vec::new())),
        Arc::new(VectorDB::new(&rara_dir.join("lancedb").to_string_lossy())),
        session_manager,
        workspace,
    );
    agent.set_execution_mode(AgentExecutionMode::Plan);
    agent
}

#[test]
fn shallow_initial_plan_continues_even_after_plan_update() {
    let mut agent = new_planning_agent();
    agent.current_plan = vec![PlanStep {
        step: "Inspect code".to_string(),
        status: PlanStepStatus::Pending,
    }];

    assert!(agent.should_continue_plan_without_tools(true, false, true, 0,));
}

#[test]
fn missing_minimum_review_evidence_continues_without_plan_update() {
    let mut agent = new_planning_agent();
    agent.inspection_progress.source_reads = 1;

    assert!(agent.should_continue_plan_without_tools(false, false, true, 1,));
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

fn assert_no_unresolved_tool_uses(history: &[crate::agent::Message]) {
    let mut pending = Vec::new();
    for message in history {
        if let Some(items) = message.content.as_array() {
            for item in items {
                match item.get("type").and_then(serde_json::Value::as_str) {
                    Some("tool_use") if message.role == "assistant" => {
                        if let Some(id) = item.get("id").and_then(serde_json::Value::as_str) {
                            pending.push(id.to_string());
                        }
                    }
                    Some("tool_result") if message.role == "user" => {
                        if let Some(id) =
                            item.get("tool_use_id").and_then(serde_json::Value::as_str)
                        {
                            if let Some(pos) =
                                pending.iter().position(|pending_id| pending_id == id)
                            {
                                pending.remove(pos);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    assert!(pending.is_empty(), "unresolved tool uses: {pending:?}");
}
