use std::sync::Arc;

use tempfile::tempdir;

use crate::agent::{Agent, AgentExecutionMode, BashApprovalMode, Message, PlanStep, PlanStepStatus};
use crate::config::ConfigManager;
use crate::oauth::OAuthManager;
use crate::tui::state::{OAuthLoginMode, TuiApp};
use crate::tool::ToolManager;
use crate::vectordb::VectorDB;
use crate::workspace::WorkspaceMemory;
use crate::session::SessionManager;
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
    assert!(!should_suggest_planning_mode(&app, "Fix the typo in README."));
    assert!(!should_suggest_planning_mode(&app, "What does this function do?"));
}

#[test]
fn browser_oauth_is_rejected_before_task_start_in_ssh() {
    let temp = tempdir().unwrap();
    let old_ssh = std::env::var_os("SSH_CONNECTION");
    std::env::set_var("SSH_CONNECTION", "test");

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

    if let Some(value) = old_ssh {
        std::env::set_var("SSH_CONNECTION", value);
    } else {
        std::env::remove_var("SSH_CONNECTION");
    }
}

#[test]
fn merge_rebuilt_agent_preserves_session_and_turn_state() {
    let temp = tempdir().unwrap();
    let workspace_root = temp.path().join("workspace");
    let rara_dir = workspace_root.join(".rara");
    std::fs::create_dir_all(rara_dir.join("rollouts")).expect("rollouts");
    std::fs::create_dir_all(rara_dir.join("sessions")).expect("sessions");
    std::fs::create_dir_all(rara_dir.join("tool-results")).expect("tool results");

    let workspace = Arc::new(WorkspaceMemory::from_paths(workspace_root.clone(), rara_dir.clone()));
    let session_manager = Arc::new(SessionManager {
        storage_dir: rara_dir.join("rollouts"),
        legacy_storage_dir: rara_dir.join("sessions"),
    });
    let backend = Arc::new(crate::llm::MockLlm);

    let mut previous = Agent::new(
        ToolManager::new(),
        backend.clone(),
        Arc::new(VectorDB::new(&rara_dir.join("lancedb").display().to_string())),
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
    previous.compact_state.compaction_count = 2;

    let rebuilt = Agent::new(
        ToolManager::new(),
        backend,
        Arc::new(VectorDB::new(&rara_dir.join("other-lancedb").display().to_string())),
        session_manager,
        workspace,
    );

    let merged = merge_rebuilt_agent(rebuilt, previous);

    assert_eq!(merged.session_id, "session-keep");
    assert_eq!(merged.history.len(), 1);
    assert_eq!(merged.total_input_tokens, 123);
    assert_eq!(merged.total_output_tokens, 45);
    assert_eq!(merged.execution_mode, AgentExecutionMode::Plan);
    assert_eq!(merged.bash_approval_mode, BashApprovalMode::Suggestion);
    assert_eq!(merged.current_plan.len(), 1);
    assert_eq!(merged.compact_state.compaction_count, 2);
}
