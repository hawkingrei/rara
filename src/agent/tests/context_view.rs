use super::support::{test_runtime_storage, SequencedBackend};
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
        Arc::new(VectorDB::new(&rara_dir.join("lancedb").display().to_string())),
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
    agent.compact_state.last_compaction_recent_files =
        vec!["src/main.rs".to_string(), "src/runtime_context.rs".to_string()];
    agent.compact_state.last_compaction_boundary = Some(crate::agent::CompactBoundaryMetadata {
        version: 3,
        before_tokens: 5000,
        recent_file_count: 2,
    });
    agent.history.push(Message {
        role: "user".to_string(),
        content: json!([{"type":"text","text":"hello"}]),
    });

    let runtime = agent.shared_runtime_context();

    assert_eq!(runtime.history_len, 1);
    assert_eq!(runtime.total_input_tokens, 11);
    assert_eq!(runtime.total_output_tokens, 7);
    assert_eq!(runtime.prompt.base_prompt_kind, "default");
    assert!(runtime
        .prompt
        .section_keys
        .contains(&"append_system_prompt".to_string()));
    assert!(runtime
        .prompt
        .warnings
        .contains(&"missing prompt file".to_string()));
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
}
