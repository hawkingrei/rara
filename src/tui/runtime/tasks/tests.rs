use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use tempfile::tempdir;
use tokio::sync::mpsc;

use crate::agent::{
    Agent, AgentExecutionMode, BashApprovalMode, Message, PlanStep, PlanStepStatus,
};
use crate::config::ConfigManager;
use crate::oauth::OAuthManager;
use crate::prompt::PromptRuntimeConfig;
use crate::session::SessionManager;
use crate::tool::ToolManager;
use crate::tui::state::{
    OAuthLoginMode, RunningTask, RuntimePhase, TaskCompletion, TaskKind, TuiApp,
};
use crate::vectordb::VectorDB;
use crate::workspace::WorkspaceMemory;
use serde_json::json;

use super::{
    emit_query_heartbeat, merge_rebuilt_agent, request_running_task_cancellation,
    should_suggest_planning_mode, start_oauth_task, try_start_queued_follow_up,
};

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

#[tokio::test]
async fn queued_follow_ups_start_as_one_multiline_turn() {
    let temp = tempdir().unwrap();
    let workspace_root = temp.path().join("workspace");
    let rara_dir = workspace_root.join(".rara");
    std::fs::create_dir_all(rara_dir.join("rollouts")).expect("rollouts");
    std::fs::create_dir_all(rara_dir.join("sessions")).expect("sessions");
    std::fs::create_dir_all(rara_dir.join("tool-results")).expect("tool results");

    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    app.queue_follow_up_message("first line");
    app.queue_follow_up_message("second line");

    let workspace = Arc::new(WorkspaceMemory::from_paths(
        workspace_root.clone(),
        rara_dir.clone(),
    ));
    let session_manager = Arc::new(SessionManager {
        storage_dir: rara_dir.join("rollouts"),
        legacy_storage_dir: rara_dir.join("sessions"),
    });
    let agent = Agent::new(
        ToolManager::new(),
        Arc::new(crate::llm::MockLlm),
        Arc::new(VectorDB::new(
            &rara_dir.join("lancedb").display().to_string(),
        )),
        session_manager,
        workspace,
    );
    let mut agent_slot = Some(agent);

    try_start_queued_follow_up(&mut app, &mut agent_slot);

    assert_eq!(app.queued_follow_up_count(), 0);
    assert!(app.running_task.is_some());
    assert_eq!(app.active_turn.entries.len(), 1);
    assert_eq!(app.active_turn.entries[0].role, "You");
    assert_eq!(
        app.active_turn.entries[0].message,
        "first line\n\nsecond line"
    );

    if let Some(task) = app.running_task.take() {
        task.handle.abort();
    }
}

#[tokio::test]
async fn query_heartbeat_preserves_running_tool_phase() {
    let temp = tempdir().unwrap();
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    let (_sender, receiver) = mpsc::unbounded_channel();
    let handle = tokio::spawn(std::future::pending::<TaskCompletion>());
    app.running_task = Some(RunningTask {
        kind: TaskKind::Query,
        receiver,
        handle,
        started_at: Instant::now() - Duration::from_secs(3),
        next_heartbeat_after_secs: 0,
        cancellation_token: None,
        cancellation_requested: false,
    });
    app.set_runtime_phase(
        RuntimePhase::RunningTool,
        Some("streaming bash output".into()),
    );

    emit_query_heartbeat(&mut app);

    assert_eq!(app.runtime_phase, RuntimePhase::RunningTool);
    assert_eq!(
        app.runtime_phase_detail.as_deref(),
        Some("streaming bash output · 3s elapsed")
    );
    if let Some(task) = app.running_task.take() {
        task.handle.abort();
    }
}

#[tokio::test]
async fn query_cancellation_sets_running_task_token() {
    let temp = tempdir().unwrap();
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    let (_sender, receiver) = mpsc::unbounded_channel();
    let token = Arc::new(AtomicBool::new(false));
    let handle = tokio::spawn(std::future::pending::<TaskCompletion>());
    app.running_task = Some(RunningTask {
        kind: TaskKind::Query,
        receiver,
        handle,
        started_at: Instant::now(),
        next_heartbeat_after_secs: 2,
        cancellation_token: Some(token.clone()),
        cancellation_requested: false,
    });

    request_running_task_cancellation(&mut app);

    assert!(token.load(Ordering::SeqCst));
    assert!(app
        .running_task
        .as_ref()
        .is_some_and(|task| task.cancellation_requested));
    assert_eq!(app.runtime_phase, RuntimePhase::ProcessingResponse);
    assert_eq!(
        app.runtime_phase_detail.as_deref(),
        Some("cancelling query")
    );

    if let Some(task) = app.running_task.take() {
        task.handle.abort();
    }
}
