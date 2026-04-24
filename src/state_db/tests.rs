use super::{
    PersistedCompactState, PersistedInteraction, PersistedPlanStep, PersistedPromptRuntimeState,
    PersistedTurnEntry, StateDb,
};
use anyhow::Result;
use rusqlite::Connection;
use tempfile::tempdir;

#[test]
fn persists_metadata_and_rollout_artifact() -> Result<()> {
    let temp = tempdir()?;
    let db = StateDb::new_for_root_dir(temp.path().join(".rara"))?;
    db.upsert_session(
        "session-1",
        "/tmp/workspace",
        "main",
        "ollama",
        "gemma4:e4b",
        Some("http://localhost:11434"),
        "execute",
        "suggestion",
        Some("Inspect the repository and summarize issues."),
        &PersistedPromptRuntimeState::default(),
        4,
        3,
        &PersistedCompactState::default(),
    )?;
    db.replace_plan_steps(
        "session-1",
        &[PersistedPlanStep {
            step_index: 0,
            status: "in_progress".to_string(),
            step: "Inspect src/main.rs".to_string(),
        }],
    )?;
    db.replace_interactions(
        "session-1",
        &[PersistedInteraction {
            kind: "approval".to_string(),
            status: "completed".to_string(),
            title: "Approval Completed".to_string(),
            summary: "run once".to_string(),
            payload: None,
        }],
    )?;
    let summary = db.persist_turn(
        "session-1",
        0,
        &[
            PersistedTurnEntry {
                role: "You".to_string(),
                message: "hello".to_string(),
            },
            PersistedTurnEntry {
                role: "Agent".to_string(),
                message: "world".to_string(),
            },
        ],
    )?;

    let verify = Connection::open(db.path())?;
    let session_count: i64 = verify.query_row(
        "SELECT COUNT(*) FROM sessions WHERE id = 'session-1'",
        [],
        |row| row.get(0),
    )?;
    let turn_count: i64 = verify.query_row(
        "SELECT COUNT(*) FROM turns WHERE session_id = 'session-1'",
        [],
        |row| row.get(0),
    )?;
    let plan_count: i64 = verify.query_row(
        "SELECT COUNT(*) FROM plan_steps WHERE session_id = 'session-1'",
        [],
        |row| row.get(0),
    )?;
    let interaction_count: i64 = verify.query_row(
        "SELECT COUNT(*) FROM interactions WHERE session_id = 'session-1'",
        [],
        |row| row.get(0),
    )?;

    assert_eq!(session_count, 1);
    assert_eq!(turn_count, 1);
    assert_eq!(plan_count, 1);
    assert_eq!(interaction_count, 1);
    assert_eq!(summary.event_count, 2);

    let artifact = db.rollout_root().join("session-1").join("000000.json");
    assert!(artifact.exists());
    let loaded = db.load_turn_entries("session-1", 0)?;
    assert_eq!(loaded.len(), 2);
    Ok(())
}

#[test]
fn persists_interaction_payloads_for_restore() -> Result<()> {
    let temp = tempdir()?;
    let db = StateDb::new_for_root_dir(temp.path().join(".rara"))?;
    db.upsert_session(
        "session-2",
        "/tmp/workspace",
        "main",
        "ollama",
        "gemma4",
        Some("http://localhost:11434"),
        "execute",
        "suggestion",
        None,
        &PersistedPromptRuntimeState::default(),
        2,
        1,
        &PersistedCompactState::default(),
    )?;
    db.replace_interactions(
        "session-2",
        &[PersistedInteraction {
            kind: "approval".to_string(),
            status: "pending".to_string(),
            title: "Pending Approval".to_string(),
            summary: "cargo test".to_string(),
            payload: Some(serde_json::json!({
                "tool_use_id": "tool-42",
                "command": "cargo test",
                "allow_net": true
            })),
        }],
    )?;

    let interactions = db.load_interactions("session-2")?;
    assert_eq!(interactions.len(), 1);
    let payload = interactions[0].payload.as_ref().expect("payload");
    assert_eq!(
        payload.get("tool_use_id").and_then(|v| v.as_str()),
        Some("tool-42")
    );
    assert_eq!(
        payload.get("command").and_then(|v| v.as_str()),
        Some("cargo test")
    );
    assert_eq!(payload.get("allow_net").and_then(|v| v.as_bool()), Some(true));
    Ok(())
}

#[test]
fn persists_structured_approval_payloads_for_restore() -> Result<()> {
    let temp = tempdir()?;
    let db = StateDb::new_for_root_dir(temp.path().join(".rara"))?;
    db.upsert_session(
        "session-structured",
        "/tmp/workspace",
        "main",
        "ollama",
        "gemma4",
        Some("http://localhost:11434"),
        "execute",
        "suggestion",
        None,
        &PersistedPromptRuntimeState::default(),
        2,
        1,
        &PersistedCompactState::default(),
    )?;
    db.replace_interactions(
        "session-structured",
        &[PersistedInteraction {
            kind: "approval".to_string(),
            status: "pending".to_string(),
            title: "Pending Approval".to_string(),
            summary: "cargo check --workspace".to_string(),
            payload: Some(serde_json::json!({
                "tool_use_id": "tool-99",
                "program": "cargo",
                "args": ["check", "--workspace"],
                "cwd": "/tmp/workspace",
                "env": { "RUST_LOG": "debug" },
                "allow_net": false
            })),
        }],
    )?;

    let interactions = db.load_interactions("session-structured")?;
    assert_eq!(interactions.len(), 1);
    let payload = interactions[0].payload.as_ref().expect("payload");
    assert_eq!(
        payload.get("tool_use_id").and_then(|v| v.as_str()),
        Some("tool-99")
    );
    assert_eq!(
        payload.get("program").and_then(|v| v.as_str()),
        Some("cargo")
    );
    assert_eq!(
        payload
            .get("args")
            .and_then(|v| v.as_array())
            .and_then(|v| v.first())
            .and_then(|v| v.as_str()),
        Some("check")
    );
    assert_eq!(
        payload.get("cwd").and_then(|v| v.as_str()),
        Some("/tmp/workspace")
    );
    assert_eq!(
        payload
            .get("env")
            .and_then(|v| v.get("RUST_LOG"))
            .and_then(|v| v.as_str()),
        Some("debug")
    );
    assert_eq!(payload.get("allow_net").and_then(|v| v.as_bool()), Some(false));
    Ok(())
}

#[test]
fn persists_compact_state_for_restore() -> Result<()> {
    let temp = tempdir()?;
    let db = StateDb::new_for_root_dir(temp.path().join(".rara"))?;
    db.upsert_session(
        "session-compact",
        "/tmp/workspace",
        "main",
        "ollama",
        "gemma4",
        Some("http://localhost:11434"),
        "execute",
        "suggestion",
        None,
        &PersistedPromptRuntimeState::default(),
        6,
        2,
        &PersistedCompactState {
            compaction_count: 3,
            last_compaction_before_tokens: Some(12_000),
            last_compaction_after_tokens: Some(4_200),
            last_compaction_recent_file_count: Some(2),
            last_compaction_boundary_version: Some(1),
        },
    )?;

    let compact_state = db.load_session_compact_state("session-compact")?;
    assert_eq!(compact_state.compaction_count, 3);
    assert_eq!(compact_state.last_compaction_before_tokens, Some(12_000));
    assert_eq!(compact_state.last_compaction_after_tokens, Some(4_200));
    assert_eq!(compact_state.last_compaction_recent_file_count, Some(2));
    assert_eq!(compact_state.last_compaction_boundary_version, Some(1));

    let threads = db.list_recent_thread_summaries(5)?;
    let summary = threads
        .iter()
        .find(|item| item.session_id == "session-compact")
        .expect("recent thread summary");
    assert_eq!(summary.compaction_count, 3);
    assert_eq!(summary.last_compaction_before_tokens, Some(12_000));
    assert_eq!(summary.last_compaction_after_tokens, Some(4_200));
    assert_eq!(summary.last_compaction_recent_file_count, Some(2));
    assert_eq!(summary.last_compaction_boundary_version, Some(1));

    let recent_threads = db.list_recent_thread_records(5)?;
    let recent = recent_threads
        .iter()
        .find(|item| item.session_id == "session-compact")
        .expect("recent thread record");
    assert_eq!(recent.cwd, "/tmp/workspace");
    assert_eq!(recent.provider, "ollama");
    assert_eq!(recent.model, "gemma4");
    assert_eq!(recent.agent_mode, "execute");
    assert_eq!(recent.bash_approval, "suggestion");
    assert_eq!(recent.compaction_count, 3);
    assert_eq!(recent.last_compaction_after_tokens, Some(4_200));
    Ok(())
}

#[test]
fn persists_session_runtime_state_for_restore() -> Result<()> {
    let temp = tempdir()?;
    let db = StateDb::new_for_root_dir(temp.path().join(".rara"))?;
    db.upsert_session(
        "session-runtime",
        "/tmp/workspace",
        "main",
        "codex",
        "gpt-5.4",
        Some("https://chatgpt.com/backend-api/codex"),
        "plan",
        "always",
        Some("Restore should rebuild the same context surface."),
        &PersistedPromptRuntimeState {
            append_system_prompt: Some("appendix".to_string()),
            warnings: vec!["missing custom prompt file".to_string()],
        },
        8,
        3,
        &PersistedCompactState::default(),
    )?;

    let runtime = db
        .load_session_runtime_state("session-runtime")?
        .expect("session runtime state");
    assert_eq!(runtime.agent_mode, "plan");
    assert_eq!(runtime.bash_approval, "always");
    assert_eq!(
        runtime.prompt_runtime.append_system_prompt.as_deref(),
        Some("appendix")
    );
    assert_eq!(
        runtime.prompt_runtime.warnings,
        vec!["missing custom prompt file".to_string()]
    );
    Ok(())
}
