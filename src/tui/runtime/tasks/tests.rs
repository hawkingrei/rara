use std::sync::Arc;

use tempfile::tempdir;

use crate::agent::{
    Agent, AgentExecutionMode, BashApprovalMode, Message, PlanStep, PlanStepStatus,
};
use crate::config::ConfigManager;
use crate::oauth::OAuthManager;
use crate::prompt::PromptRuntimeConfig;
use crate::session::SessionManager;
use crate::tool::ToolManager;
use crate::tui::state::{OAuthLoginMode, TuiApp};
use crate::vectordb::VectorDB;
use crate::workspace::WorkspaceMemory;
use serde_json::json;

use super::{merge_rebuilt_agent, should_suggest_planning_mode, start_oauth_task};

#[test]
fn suggests_planning_for_repo_review_requests() {
    let temp = tempdir().unwrap();
    let app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    assert!(should_suggest_planning_mode(
        &app,
        "看一下代码，并提出修改建议"
    ));
    assert!(should_suggest_planning_mode(
        &app,
        "Review this repository and propose architectural improvements."
    ));
}

#[test]
fn skips_planning_for_simple_requests() {
    let temp = tempdir().unwrap();
    let app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    assert!(!should_suggest_planning_mode(
        &app,
        "Fix the typo in README."
    ));
    assert!(!should_suggest_planning_mode(
        &app,
        "What does this function do?"
    ));
}

#[test]
fn browser_oauth_is_rejected_before_task_start_in_ssh() {
    let temp = tempdir().unwrap();
    let _ssh_env = crate::tui::terminal_ui::test_env::set_ssh_session(true);

    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    let oauth_manager = Arc::new(
        OAuthManager::new_for_config_dir(temp.path().join(".rara")).expect("oauth manager"),
    );

    start_oauth_task(&mut app, oauth_manager, OAuthLoginMode::Browser);

    assert!(app.running_task.is_none());
    assert!(app
        .notice
        .as_deref()
        .is_some_and(|value| value.contains("Browser login is unavailable")));
}

#[test]
fn merge_rebuilt_agent_preserves_session_and_turn_state() {
    let temp = tempdir().unwrap();
    let workspace_root = temp.path().join("workspace");
    let rara_dir = workspace_root.join(".rara");
    std::fs::create_dir_all(rara_dir.join("rollouts")).expect("rollouts");
    std::fs::create_dir_all(rara_dir.join("sessions")).expect("sessions");
    std::fs::create_dir_all(rara_dir.join("tool-results")).expect("tool results");

    let workspace = Arc::new(WorkspaceMemory::from_paths(
        workspace_root.clone(),
        rara_dir.clone(),
    ));
    let session_manager = Arc::new(SessionManager {
        storage_dir: rara_dir.join("rollouts"),
        legacy_storage_dir: rara_dir.join("sessions"),
    });
    let backend = Arc::new(crate::llm::MockLlm);

    let mut previous = Agent::new(
        ToolManager::new(),
        backend.clone(),
        Arc::new(VectorDB::new(
            &rara_dir.join("lancedb").display().to_string(),
        )),
        session_manager.clone(),
        workspace.clone(),
    );
    previous.session_id = "session-keep".to_string();
    previous.history.push(Message {
        role: "user".into(),
        content: json!([{"type":"text","text":"keep history"}]),
    });
    previous.total_input_tokens = 123;
    previous.total_output_tokens = 45;
    previous.execution_mode = AgentExecutionMode::Plan;
    previous.bash_approval_mode = BashApprovalMode::Suggestion;
    previous.current_plan = vec![PlanStep {
        step: "Keep session continuity".into(),
        status: PlanStepStatus::InProgress,
    }];
    previous.plan_explanation = Some("Do not reset the session during model switch.".into());
    previous.compact_state.estimated_history_tokens = 1_200;
    previous.compact_state.context_window_tokens = Some(8_192);
    previous.compact_state.compact_threshold_tokens = 7_000;
    previous.compact_state.reserved_output_tokens = 1_024;
    previous.compact_state.compaction_count = 2;
    previous.compact_state.last_compaction_before_tokens = Some(5_000);
    previous.compact_state.last_compaction_after_tokens = Some(2_100);
    previous.compact_state.last_compaction_recent_files = vec!["src/main.rs".into()];
    previous.compact_state.last_compaction_boundary = Some(crate::agent::CompactBoundaryMetadata {
        version: 1,
        before_tokens: 5_000,
        recent_file_count: 1,
    });
    previous.set_prompt_config(PromptRuntimeConfig {
        append_system_prompt: Some("keep appendix".to_string()),
        warnings: vec!["missing custom prompt".to_string()],
        ..PromptRuntimeConfig::default()
    });

    let mut rebuilt = Agent::new(
        ToolManager::new(),
        backend,
        Arc::new(VectorDB::new(
            &rara_dir.join("other-lancedb").display().to_string(),
        )),
        session_manager,
        workspace,
    );
    rebuilt.compact_state.context_window_tokens = Some(200_000);
    rebuilt.compact_state.compact_threshold_tokens = 180_000;
    rebuilt.compact_state.reserved_output_tokens = 8_192;

    let merged = merge_rebuilt_agent(rebuilt, previous);

    assert_eq!(merged.session_id, "session-keep");
    assert_eq!(merged.history.len(), 1);
    assert_eq!(merged.total_input_tokens, 123);
    assert_eq!(merged.total_output_tokens, 45);
    assert_eq!(merged.execution_mode, AgentExecutionMode::Plan);
    assert_eq!(merged.bash_approval_mode, BashApprovalMode::Suggestion);
    assert_eq!(merged.current_plan.len(), 1);
    assert_eq!(merged.compact_state.estimated_history_tokens, 1_200);
    assert_eq!(merged.compact_state.compaction_count, 2);
    assert_eq!(
        merged.compact_state.last_compaction_before_tokens,
        Some(5_000)
    );
    assert_eq!(
        merged.compact_state.last_compaction_after_tokens,
        Some(2_100)
    );
    assert_eq!(
        merged.compact_state.last_compaction_recent_files,
        vec!["src/main.rs".to_string()]
    );
    assert_eq!(merged.compact_state.context_window_tokens, Some(200_000));
    assert_eq!(merged.compact_state.compact_threshold_tokens, 180_000);
    assert_eq!(merged.compact_state.reserved_output_tokens, 8_192);
    assert_eq!(
        merged.prompt_config().append_system_prompt.as_deref(),
        Some("keep appendix")
    );
    assert_eq!(
        merged.prompt_config().warnings,
        vec!["missing custom prompt".to_string()]
    );
}
