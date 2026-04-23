use std::sync::Arc;

use tempfile::tempdir;

use crate::config::ConfigManager;
use crate::oauth::OAuthManager;
use crate::tui::state::{OAuthLoginMode, TuiApp};

use super::{should_suggest_planning_mode, start_oauth_task};

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
