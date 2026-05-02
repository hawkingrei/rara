use std::time::Instant;

use ratatui::{layout::Rect, style::Color};
use tempfile::tempdir;
use tokio::sync::mpsc;

use crate::config::ConfigManager;
use crate::tui::state::{
    InteractionKind, PendingInteractionSnapshot, RunningTask, RuntimePhase, RuntimeSnapshot,
    TaskCompletion, TaskKind, TuiApp,
};

use super::{
    activity_status_line, composer_hint, composer_hint_line, footer_summary_text,
    wrapped_text_cursor_position, wrapped_text_rows,
};

#[test]
fn footer_summary_text_prefers_minimal_idle_context() {
    let temp = tempdir().unwrap();
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    app.snapshot = RuntimeSnapshot {
        estimated_history_tokens: 1234,
        context_window_tokens: Some(32768),
        ..RuntimeSnapshot::default()
    };

    let rendered = footer_summary_text(&app);
    assert_eq!(rendered, "ctx~=1234/32768");
}

#[test]
fn footer_summary_text_shows_tokens_only_while_busy() {
    let temp = tempdir().unwrap();
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    app.runtime_phase = RuntimePhase::ProcessingResponse;
    app.snapshot = RuntimeSnapshot {
        estimated_history_tokens: 2048,
        context_window_tokens: Some(32768),
        total_input_tokens: 111,
        total_output_tokens: 22,
        ..RuntimeSnapshot::default()
    };

    let rendered = footer_summary_text(&app);
    assert_eq!(rendered, "ctx~=2048/32768  tokens=111 in / 22 out");
    assert!(!rendered.contains("history="));
    assert!(!rendered.contains("local="));
    assert!(!rendered.contains("key="));
}

#[test]
fn activity_status_line_prefers_pending_interactions() {
    let temp = tempdir().unwrap();
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    app.snapshot
        .pending_interactions
        .push(PendingInteractionSnapshot {
            kind: InteractionKind::PlanApproval,
            title: "Approve plan".into(),
            summary: "ready".into(),
            options: Vec::new(),
            note: None,
            approval: None,
            source: None,
        });

    let (label, _, detail) = activity_status_line(&app);
    assert_eq!(label, "Plan Approval");
    assert!(detail.contains("start implementation"));
    assert!(detail.contains("continue planning"));
}

#[test]
fn pending_interaction_hint_takes_priority_over_queued_follow_up() {
    let temp = tempdir().unwrap();
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    app.queue_follow_up_message("then review the diff");
    app.snapshot
        .pending_interactions
        .push(PendingInteractionSnapshot {
            kind: InteractionKind::Approval,
            title: "Shell Approval".into(),
            summary: "git diff origin/main -- src/context/assembler.rs".into(),
            options: Vec::new(),
            note: None,
            approval: None,
            source: None,
        });

    let hint = composer_hint(&app);
    assert!(hint.contains("1 allow once"));
    assert!(!hint.contains("queued follow-up"));
}

#[test]
fn activity_status_line_renders_warning_notice_in_yellow() {
    let temp = tempdir().unwrap();
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    app.notice = Some(
        "Warning: something went wrong"
            .to_string(),
    );

    let (label, color, _detail) = activity_status_line(&app);
    assert_eq!(label, "Warning");
    assert_eq!(color, Color::Yellow);
}

#[test]
fn composer_hint_line_includes_repo_context_when_present() {
    let temp = tempdir().unwrap();
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    app.set_repo_context_hint("github.com/user/repo PR #42".to_string());

    let line = composer_hint_line(&app);
    let text: String = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect();
    assert!(text.contains("github.com/user/repo PR #42"));
}

#[test]
fn composer_hint_line_omits_repo_context_when_none() {
    let temp = tempdir().unwrap();
    let app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");

    let line = composer_hint_line(&app);
    let text: String = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect();
    assert!(!text.contains("PR #"));
}

#[test]
fn composer_hint_reflects_running_query_background_task() {
    let temp = tempdir().unwrap();
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    app.running_task = Some(RunningTask {
        id: "t1".into(),
        kind: TaskKind::Query,
        command: None,
        started_at: Instant::now(),
        completed: None,
        pty_session: None,
        background_task: None,
    });

    let hint = composer_hint(&app);
    assert!(hint.contains("Enter queue"));
    assert!(hint.contains("Esc cancel"));
}

#[test]
fn composer_hint_reflects_rebuild_background_task() {
    let temp = tempdir().unwrap();
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    app.running_task = Some(RunningTask {
        id: "t2".into(),
        kind: TaskKind::Rebuild,
        command: None,
        started_at: Instant::now(),
        completed: None,
        pty_session: None,
        background_task: None,
    });

    let hint = composer_hint(&app);
    assert!(hint.contains("Enter queue"));
    assert!(!hint.contains("Esc cancel"));
}

#[test]
fn composer_hint_reflects_pending_planning_suggestion() {
    let temp = tempdir().unwrap();
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    app.set_pending_planning_suggestion("Let's plan this out first".to_string());

    let hint = composer_hint(&app);
    assert!(hint.contains("1 enter planning mode"));
    assert!(hint.contains("2 continue in execute mode"));
}

#[test]
fn composer_hint_reflects_plan_mode() {
    let temp = tempdir().unwrap();
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    app.set_agent_execution_mode_label("plan".to_string());

    let hint = composer_hint(&app);
    assert!(hint.contains("planning mode"));
    assert!(hint.contains("read-only planning"));
}

#[test]
fn editor_cursor_position_starts_at_one_one_offset() {
    let area = Rect {
        x: 0,
        y: 0,
        width: 80,
        height: 24,
    };
    let (x, y) = wrapped_text_cursor_position("", 0, area, None, None);
    assert_eq!(x, 1);
    assert_eq!(y, 1);
}

#[test]
fn wrapped_text_rows_counts_composer_content_lines() {
    // Includes indent and subsequent indent in width calculations
    let rows = wrapped_text_rows("hello world", 80, Some("› "), Some("  "));
    assert_eq!(rows, 1);

    let long = "a".repeat(90);
    let rows = wrapped_text_rows(&long, 30, Some("› "), Some("  "));
    assert!(rows > 1);
}

#[test]
fn wrapped_text_rows_handles_explicit_newlines() {
    let rows = wrapped_text_rows("line one\nline two", 50, Some("› "), Some("  "));
    assert_eq!(rows, 2);
}

#[test]
fn wrapped_text_rows_handles_empty_text() {
    let rows = wrapped_text_rows("", 50, Some("› "), Some("  "));
    assert_eq!(rows, 1);
}
