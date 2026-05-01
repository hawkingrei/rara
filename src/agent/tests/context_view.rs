use super::support::{SequencedBackend, test_runtime_storage};
use crate::agent::{Agent, AgentExecutionMode, Message, PlanStep, PlanStepStatus};
use crate::llm::{ContentBlock, LlmResponse};
use crate::prompt::PromptRuntimeConfig;
use crate::tool::ToolManager;
use crate::vectordb::VectorDB;
use serde_json::json;
use std::sync::Arc;

#[test]
fn shared_runtime_context_collects_prompt_plan_and_compaction_state() {
    let (_temp, session_manager, workspace, rara_dir) = test_runtime_storage();
    std::fs::write(
        rara_dir.join("memory.md"),
        "# Team Notes\n\nPrefer the shared bootstrap path.\n",
    )
    .expect("write workspace memory");
    let backend = Arc::new(SequencedBackend::new(vec![LlmResponse {
        content: vec![ContentBlock::Text {
            text: "ok".to_string(),
        }],
        stop_reason: Some("end_turn".to_string()),
        usage: None,
    }]));

    let mut agent = Agent::new(
        ToolManager::new(),
        backend,
        Arc::new(VectorDB::new(
            &rara_dir.join("lancedb").display().to_string(),
        )),
        session_manager,
        workspace,
    );
    agent.set_prompt_config(PromptRuntimeConfig {
        append_system_prompt: Some("appendix".to_string()),
        warnings: vec!["missing prompt file".to_string()],
        ..PromptRuntimeConfig::default()
    });
    agent.execution_mode = AgentExecutionMode::Plan;
    agent.current_plan = vec![
        PlanStep {
            step: "inspect auth flow".to_string(),
            status: PlanStepStatus::Completed,
        },
        PlanStep {
            step: "replace bootstrap path".to_string(),
            status: PlanStepStatus::Pending,
        },
    ];
    agent.plan_explanation = Some("Prefer one shared bootstrap path.".to_string());
    agent.total_input_tokens = 11;
    agent.total_output_tokens = 7;
    agent.compact_state.estimated_history_tokens = 1234;
    agent.compact_state.context_window_tokens = Some(8192);
    agent.compact_state.compact_threshold_tokens = 7000;
    agent.compact_state.reserved_output_tokens = 1024;
    agent.compact_state.compaction_count = 2;
    agent.compact_state.last_compaction_before_tokens = Some(5000);
    agent.compact_state.last_compaction_after_tokens = Some(2100);
    agent.compact_state.last_compaction_recent_files = vec![
        "src/main.rs".to_string(),
        "src/runtime_context.rs".to_string(),
    ];
    agent.compact_state.last_compaction_boundary = Some(crate::agent::CompactBoundaryMetadata {
        version: 3,
        before_tokens: 5000,
        recent_file_count: 2,
    });
    agent.history.push(Message {
        role: "user".to_string(),
        content: json!([{"type":"text","text":"hello"}]),
    });
    agent.history.push(Message {
        role: "system".to_string(),
        content: json!([{
            "type": "compact_boundary",
            "version": 3,
            "before_tokens": 5000,
            "recent_file_count": 2
        }]),
    });
    agent.history.push(Message {
        role: "system".to_string(),
        content: json!([{
            "type": "compacted_summary",
            "text": "User Intent\n- finish the refactor"
        }]),
    });
    agent.history.push(Message {
        role: "system".to_string(),
        content: json!([{
            "type": "recent_files",
            "files": [
                "src/main.rs",
                "src/runtime_context.rs"
            ]
        }]),
    });
    agent.history.push(Message {
        role: "system".to_string(),
        content: json!([{
            "type": "recent_file_excerpts",
            "files": [{
                "path": "src/main.rs",
                "line_start": 1,
                "line_end": 3,
                "snippet": "code"
            }]
        }]),
    });
    agent.history.push(Message {
        role: "assistant".to_string(),
        content: json!([
            {
                "type": "tool_use",
                "id": "tool-retrieve-1",
                "name": "retrieve_experience",
                "input": { "query": "bootstrap contract" }
            },
            {
                "type": "tool_use",
                "id": "tool-retrieve-2",
                "name": "retrieve_session_context",
                "input": { "query": "previous auth flow" }
            }
        ]),
    });
    agent.history.push(Message {
        role: "user".to_string(),
        content: json!([
            {
                "type": "tool_result",
                "tool_use_id": "tool-retrieve-1",
                "content": "Tool retrieve_experience completed with relevant_experiences.\nPayload:\n{\n  \"relevant_experiences\": [\n    \"Prefer one shared bootstrap path.\",\n    \"Keep session restore aligned with direct execution.\"\n  ]\n}"
            },
            {
                "type": "tool_result",
                "tool_use_id": "tool-retrieve-2",
                "content": "Tool retrieve_session_context completed with status, summary.\nPayload:\n{\n  \"status\": \"ok\",\n  \"summary\": \"Auth picker already moved behind the shared runtime bootstrap.\"\n}"
            }
        ]),
    });

    let runtime = agent.shared_runtime_context();

    assert_eq!(runtime.history_len, 7);
    assert_eq!(runtime.total_input_tokens, 11);
    assert_eq!(runtime.total_output_tokens, 7);
    assert_eq!(runtime.prompt.base_prompt_kind, "default");
    assert!(
        runtime
            .prompt
            .section_keys
            .contains(&"append_system_prompt".to_string())
    );
    assert_eq!(
        runtime.prompt.source_entries.len(),
        runtime.prompt.source_status_lines.len()
    );
    assert_eq!(runtime.prompt.source_entries[0].order, 1);
    assert!(!runtime.prompt.source_entries[0].inclusion_reason.is_empty());
    assert!(
        runtime
            .prompt
            .warnings
            .contains(&"missing prompt file".to_string())
    );
    assert_eq!(runtime.plan.execution_mode, "plan");
    assert_eq!(
        runtime.plan.steps,
        vec![
            ("completed".to_string(), "inspect auth flow".to_string()),
            ("pending".to_string(), "replace bootstrap path".to_string()),
        ]
    );
    assert_eq!(
        runtime.plan.explanation.as_deref(),
        Some("Prefer one shared bootstrap path.")
    );
    assert_eq!(runtime.compaction.estimated_history_tokens, 1234);
    assert_eq!(runtime.compaction.context_window_tokens, Some(8192));
    assert_eq!(runtime.compaction.last_compaction_boundary_version, Some(3));
    assert_eq!(
        runtime.compaction.last_compaction_recent_files,
        vec![
            "src/main.rs".to_string(),
            "src/runtime_context.rs".to_string()
        ]
    );
    assert_eq!(runtime.compaction.source_entries.len(), 4);
    assert_eq!(
        runtime.compaction.source_entries[0].kind,
        "compact_boundary"
    );
    assert_eq!(
        runtime.compaction.source_entries[1].kind,
        "compacted_summary"
    );
    assert_eq!(runtime.compaction.source_entries[2].kind, "recent_files");
    assert_eq!(
        runtime.compaction.source_entries[3].kind,
        "recent_file_excerpts"
    );
    assert_eq!(runtime.retrieval.entries.len(), 3);
    assert_eq!(runtime.retrieval.entries[0].kind, "workspace_memory");
    assert_eq!(runtime.retrieval.entries[0].status, "active");
    assert_eq!(runtime.retrieval.entries[1].kind, "thread_history");
    assert_eq!(runtime.retrieval.entries[1].status, "available");
    assert_eq!(runtime.retrieval.entries[2].kind, "vector_memory");
    assert_eq!(runtime.retrieval.entries[2].status, "available");
    assert_eq!(runtime.retrieval.memory_selection.selected_items.len(), 9);
    assert_eq!(
        runtime.retrieval.memory_selection.selected_items[0].kind,
        "workspace_memory"
    );
    assert!(
        runtime.retrieval.memory_selection.selected_items[0]
            .detail
            .contains(".rara/memory.md")
    );
    assert!(
        runtime.retrieval.memory_selection.selected_items[0]
            .detail
            .contains("2 non-empty lines")
    );
    assert_eq!(
        runtime.retrieval.memory_selection.selected_items[1].kind,
        "compacted_summary"
    );
    assert_eq!(
        runtime.retrieval.memory_selection.selected_items[2].kind,
        "recent_files"
    );
    assert_eq!(
        runtime.retrieval.memory_selection.selected_items[3].kind,
        "recent_file_excerpts"
    );
    assert_eq!(
        runtime.retrieval.memory_selection.selected_items[4].kind,
        "plan_explanation"
    );
    assert_eq!(
        runtime.retrieval.memory_selection.selected_items[5].kind,
        "plan_steps"
    );
    assert_eq!(
        runtime.retrieval.memory_selection.selected_items[6].kind,
        "latest_user_request"
    );
    assert_eq!(
        runtime.retrieval.memory_selection.selected_items[7].kind,
        "tool_result"
    );
    assert_eq!(
        runtime.retrieval.memory_selection.selected_items[8].kind,
        "tool_result"
    );
    assert!(
        runtime
            .retrieval
            .memory_selection
            .selected_items
            .iter()
            .all(|item| !matches!(
                item.kind.as_str(),
                "retrieved_workspace_memory" | "retrieved_thread_context"
            ))
    );
    assert_eq!(runtime.retrieval.memory_selection.available_items.len(), 2);
    assert_eq!(
        runtime.retrieval.memory_selection.available_items[0].kind,
        "thread_history"
    );
    assert_eq!(
        runtime.retrieval.memory_selection.available_items[1].kind,
        "vector_memory"
    );
    assert_eq!(runtime.retrieval.memory_selection.dropped_items.len(), 2);
    assert_eq!(
        runtime.retrieval.memory_selection.dropped_items[0].kind,
        "retrieved_workspace_memory"
    );
    assert!(
        runtime.retrieval.memory_selection.dropped_items[0]
            .detail
            .contains("recalled=2 item(s)")
    );
    assert_eq!(
        runtime.retrieval.memory_selection.dropped_items[1].kind,
        "retrieved_thread_context"
    );
    assert!(
        runtime.retrieval.memory_selection.dropped_items[1]
            .detail
            .contains("Auth picker already moved behind the shared runtime bootstrap.")
    );
    assert!(
        runtime
            .retrieval
            .memory_selection
            .selection_budget_tokens
            .is_some()
    );
    assert!(runtime.budget.stable_instructions_budget > 0);
    assert!(runtime.budget.active_turn_budget > 0);
    assert!(
        runtime
            .assembly
            .entries
            .iter()
            .any(|entry| entry.layer == "stable_instructions" && entry.injected)
    );
    assert!(
        runtime
            .assembly
            .entries
            .iter()
            .any(|entry| entry.layer == "compacted_history" && entry.injected)
    );
    assert!(
        runtime
            .assembly
            .entries
            .iter()
            .any(|entry| entry.layer == "retrieval_ready" && !entry.injected)
    );
}

#[test]
fn assemble_turn_context_matches_prompt_and_runtime_views() {
    let (_temp, session_manager, workspace, rara_dir) = test_runtime_storage();
    std::fs::write(
        rara_dir.join("memory.md"),
        "# Team Notes\n\nPrefer the shared bootstrap path.\n",
    )
    .expect("write workspace memory");
    let backend = Arc::new(SequencedBackend::new(vec![LlmResponse {
        content: vec![ContentBlock::Text {
            text: "ok".to_string(),
        }],
        stop_reason: Some("end_turn".to_string()),
        usage: None,
    }]));

    let mut agent = Agent::new(
        ToolManager::new(),
        backend,
        Arc::new(VectorDB::new(
            &rara_dir.join("lancedb").display().to_string(),
        )),
        session_manager,
        workspace,
    );
    agent.set_prompt_config(PromptRuntimeConfig {
        append_system_prompt: Some("appendix".to_string()),
        warnings: vec!["missing prompt file".to_string()],
        ..PromptRuntimeConfig::default()
    });
    agent.execution_mode = AgentExecutionMode::Plan;
    agent.current_plan = vec![PlanStep {
        step: "inspect auth flow".to_string(),
        status: PlanStepStatus::Pending,
    }];
    agent.plan_explanation = Some("Prefer one shared bootstrap path.".to_string());
    agent.history.push(Message {
        role: "user".to_string(),
        content: json!([{"type":"text","text":"hello"}]),
    });

    let assembled = agent.assemble_turn_context();

    assert_eq!(
        assembled.prompt.system_prompt(),
        agent.build_system_prompt()
    );
    assert_eq!(assembled.runtime, agent.shared_runtime_context());
    assert_eq!(
        assembled.runtime.prompt.append_system_prompt.as_deref(),
        Some("appendix")
    );
    assert_eq!(assembled.runtime.plan.execution_mode, "plan");
}
