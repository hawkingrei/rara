use std::sync::Arc;
use std::time::Instant;

use tempfile::tempdir;
use tokio::sync::mpsc;

use crate::agent::{Agent, AgentExecutionMode, PlanStep, PlanStepStatus};
use crate::config::ConfigManager;
use crate::llm::MockLlm;
use crate::oauth::OAuthManager;
use crate::session::SessionManager;
use crate::tool::ToolManager;
use crate::tui::state::{OAuthLoginMode, TuiApp};
use crate::tui::state::{RunningTask, TaskCompletion, TaskKind};
use crate::vectordb::VectorDB;
use crate::workspace::WorkspaceMemory;

use super::{finish_running_task_if_ready, should_suggest_planning_mode, start_oauth_task};

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

fn test_agent(root: &std::path::Path, rara_dir: &std::path::Path) -> Agent {
    Agent::new(
        ToolManager::new(),
        Arc::new(MockLlm),
        Arc::new(VectorDB::new(&rara_dir.join("lancedb").display().to_string())),
        Arc::new(SessionManager::new_for_rara_dir(rara_dir.to_path_buf()).expect("session manager")),
        Arc::new(WorkspaceMemory::from_paths(root.to_path_buf(), rara_dir.to_path_buf())),
    )
}

#[tokio::test]
async fn rebuild_keeps_transcript_and_session_continuity() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().join("repo");
    let rara_dir = root.join(".rara");
    std::fs::create_dir_all(&rara_dir).expect("mkdir rara");

    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("app");
    app.agent_execution_mode = AgentExecutionMode::Plan;
    app.push_entry("You", "keep this transcript");
    app.finalize_active_turn();

    let mut previous = test_agent(&root, &rara_dir);
    previous.session_id = "thread-123".to_string();
    previous.current_plan = vec![PlanStep {
        step: "keep continuity".to_string(),
        status: PlanStepStatus::InProgress,
    }];
    previous.plan_explanation = Some("Preserve session continuity.".to_string());
    previous.compact_state.estimated_history_tokens = 4321;
    app.sync_snapshot(&previous);

    let rebuilt = test_agent(&root, &rara_dir);
    let (_sender, receiver) = mpsc::unbounded_channel();
    app.running_task = Some(RunningTask {
        kind: TaskKind::Rebuild,
        receiver,
        handle: tokio::spawn(async move {
            TaskCompletion::Rebuild {
                result: Ok(crate::tui::state::RebuildSuccess {
                    agent: rebuilt,
                    warnings: Vec::new(),
                }),
            }
        }),
        started_at: Instant::now(),
        next_heartbeat_after_secs: 2,
    });

    let mut agent_slot = Some(previous);
    finish_running_task_if_ready(&mut app, &mut agent_slot)
        .await
        .expect("finish rebuild");

    assert_eq!(app.committed_turns.len(), 1);
    assert_eq!(app.committed_turns[0].entries[0].message, "keep this transcript");
    assert_eq!(app.snapshot.session_id, "thread-123");
    assert_eq!(app.snapshot.plan_steps.len(), 1);
    assert_eq!(app.snapshot.plan_steps[0].1, "keep continuity");
    assert_eq!(app.snapshot.estimated_history_tokens, 4321);
    assert_eq!(
        agent_slot.as_ref().expect("agent").session_id,
        "thread-123"
    );
}
