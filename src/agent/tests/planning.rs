use std::sync::Arc;

use serde_json::json;

use crate::agent::planning::{
    parse_plan_block, parse_request_user_input_block, strip_continue_inspection_control,
};
use crate::agent::{
    Agent, AgentExecutionMode, ContentBlock, PendingUserInput, PlanStep, PlanStepStatus,
    RuntimeContinuationPhase,
};
use crate::llm::{LlmResponse, TokenUsage};
use crate::session::SessionManager;
use crate::tool::ToolManager;
use crate::tool_result::ToolResultStore;
use crate::vectordb::VectorDB;
use crate::workspace::WorkspaceMemory;

use super::support::{test_runtime_storage, SequencedBackend, StubTool};

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
    let mut agent = Agent::new(
        tool_manager,
        backend.clone(),
        Arc::new(VectorDB::new("data/lancedb")),
        Arc::new(SessionManager::new().expect("session manager")),
        Arc::new(WorkspaceMemory::new().expect("workspace memory")),
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
async fn forces_final_answer_when_tool_loop_exceeds_limit() {
    let mut responses = (0..=super::super::MAX_TOOL_ROUNDS_PER_TURN)
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
    let mut agent = Agent::new(
        tool_manager,
        backend.clone(),
        Arc::new(VectorDB::new("data/lancedb")),
        Arc::new(SessionManager::new().expect("session manager")),
        Arc::new(WorkspaceMemory::new().expect("workspace memory")),
    );

    agent
        .query_with_mode("loop".to_string(), super::super::AgentOutputMode::Silent)
        .await
        .expect("query should finish with a forced final answer");

    let observed_tools = backend.observed_tools();
    assert_eq!(
        observed_tools.len(),
        super::super::MAX_TOOL_ROUNDS_PER_TURN + 2
    );
    assert!(observed_tools.last().is_some_and(|tools| tools.is_empty()));
    assert!(agent.history.last().is_some_and(|message| message
        .content
        .to_string()
        .contains("Final answer after reviewing the tool results.")));
}

#[tokio::test]
async fn returns_local_fallback_when_forced_final_answer_still_calls_tools() {
    let mut responses = (0..=super::super::MAX_TOOL_ROUNDS_PER_TURN)
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
        content: vec![ContentBlock::ToolUse {
            id: "tool-final".to_string(),
            name: "stub_tool".to_string(),
            input: json!({}),
        }],
        stop_reason: Some("tool_use".to_string()),
        usage: Some(TokenUsage::default()),
    });
    let backend = Arc::new(SequencedBackend::new(responses));

    let mut tool_manager = ToolManager::new();
    tool_manager.register(Box::new(StubTool));
    let mut agent = Agent::new(
        tool_manager,
        backend.clone(),
        Arc::new(VectorDB::new("data/lancedb")),
        Arc::new(SessionManager::new().expect("session manager")),
        Arc::new(WorkspaceMemory::new().expect("workspace memory")),
    );

    let mut events = Vec::new();
    agent
        .query_with_mode_and_events(
            "loop".to_string(),
            super::super::AgentOutputMode::Silent,
            |event| events.push(event),
        )
        .await
        .expect("query should finish with a local fallback answer");

    let observed_tools = backend.observed_tools();
    assert!(observed_tools.last().is_some_and(|tools| tools.is_empty()));
    assert!(agent.history.last().is_some_and(|message| message
        .content
        .to_string()
        .contains("Tool loop reached the")));
    assert!(agent
        .history
        .iter()
        .any(|message| message.content.to_string().contains("tool-final")));
    assert_no_unresolved_tool_uses(&agent.history);
    assert!(events.iter().any(|event| matches!(
        event,
        super::super::AgentEvent::AssistantText(text)
            if text.contains("Tool loop reached the")
    )));
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
