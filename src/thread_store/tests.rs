use anyhow::Result;
use tempfile::tempdir;

use crate::agent::Message;
use crate::session::{PersistedCompactionEvent, SessionManager};
use crate::state_db::{
    PersistedCompactState, PersistedInteraction, PersistedPlanStep, PersistedPromptRuntimeState,
    PersistedTurnEntry, StateDb,
};

use super::{RolloutItem, ThreadRecorder, ThreadRuntimeState, ThreadStore};

#[test]
fn load_thread_aggregates_history_state_and_rollout_items() -> Result<()> {
    let temp = tempdir()?;
    let rara_dir = temp.path().join(".rara");
    let session_manager = SessionManager::new_for_rara_dir(rara_dir.clone())?;
    let state_db = StateDb::new_for_root_dir(rara_dir)?;
    session_manager.save_session(
        "session-1",
        &[Message {
            role: "user".to_string(),
            content: serde_json::json!("hello"),
        }],
    )?;
    state_db.upsert_session(
        "session-1",
        "/tmp/workspace",
        "main",
        "ollama",
        "qwen3",
        None,
        "execute",
        "always",
        Some("Inspect current plan state."),
        &PersistedPromptRuntimeState::default(),
        1,
        1,
        &PersistedCompactState {
            compaction_count: 2,
            last_compaction_before_tokens: Some(8000),
            last_compaction_after_tokens: Some(2400),
            last_compaction_recent_file_count: Some(1),
            last_compaction_boundary_version: Some(3),
        },
    )?;
    state_db.replace_plan_steps(
        "session-1",
        &[PersistedPlanStep {
            step_index: 0,
            status: "in_progress".to_string(),
            step: "Inspect src/thread_store.rs".to_string(),
        }],
    )?;
    state_db.replace_interactions(
        "session-1",
        &[PersistedInteraction {
            kind: "approval".to_string(),
            status: "pending".to_string(),
            title: "Need approval".to_string(),
            summary: "cargo check".to_string(),
            payload: None,
        }],
    )?;
    state_db.persist_turn(
        "session-1",
        0,
        &[PersistedTurnEntry {
            role: "Agent".to_string(),
            message: "Investigating.".to_string(),
        }],
    )?;
    session_manager.save_compaction_event(
        "session-1",
        &PersistedCompactionEvent {
            event_index: 2,
            before_tokens: 8000,
            after_tokens: 2400,
            boundary_version: 3,
            recent_files: vec!["src/thread_store.rs".to_string()],
            summary: "Compacted earlier repository inspection.".to_string(),
        },
    )?;

    let store = ThreadStore::new(&session_manager, &state_db);
    let snapshot = store.load_thread("session-1")?;

    assert_eq!(snapshot.metadata.session_id, "session-1");
    assert_eq!(snapshot.metadata.provider, "ollama");
    assert_eq!(snapshot.history.len(), 1);
    assert_eq!(snapshot.compaction.compaction_count, 2);
    assert_eq!(
        snapshot.compaction.summary.as_deref(),
        Some("Compacted earlier repository inspection.")
    );
    assert_eq!(
        snapshot.compaction.recent_files,
        vec!["src/thread_store.rs".to_string()]
    );
    assert_eq!(snapshot.plan_explanation.as_deref(), Some("Inspect current plan state."));
    assert_eq!(snapshot.plan_steps.len(), 1);
    assert_eq!(snapshot.interactions.len(), 1);
    assert_eq!(snapshot.rollout_items.len(), 4);
    assert!(matches!(
        snapshot.rollout_items.first(),
        Some(RolloutItem::Compaction(compaction)) if compaction.compaction_count == 2
    ));
    assert!(matches!(
        snapshot.rollout_items.get(1),
        Some(RolloutItem::PlanState { explanation, steps })
            if explanation.as_deref() == Some("Inspect current plan state.") && steps.len() == 1
    ));
    assert!(matches!(
        snapshot.rollout_items.get(2),
        Some(RolloutItem::Interaction(interaction))
            if interaction.kind == "approval" && interaction.status == "pending"
    ));
    match &snapshot.rollout_items[3] {
        RolloutItem::Turn(turn) => {
            assert_eq!(turn.summary.ordinal, 0);
            assert_eq!(turn.entries.len(), 1);
            assert_eq!(turn.entries[0].message, "Investigating.");
        }
        _ => panic!("expected committed turn rollout item"),
    }

    Ok(())
}

#[test]
fn load_thread_keeps_session_without_history_file() -> Result<()> {
    let temp = tempdir()?;
    let rara_dir = temp.path().join(".rara");
    let session_manager = SessionManager::new_for_rara_dir(rara_dir.clone())?;
    let state_db = StateDb::new_for_root_dir(rara_dir)?;
    state_db.upsert_session(
        "session-missing-history",
        "/tmp/workspace",
        "main",
        "ollama",
        "qwen3",
        None,
        "execute",
        "always",
        None,
        &PersistedPromptRuntimeState::default(),
        0,
        0,
        &PersistedCompactState::default(),
    )?;

    let store = ThreadStore::new(&session_manager, &state_db);
    let snapshot = store.load_thread("session-missing-history")?;

    assert_eq!(snapshot.metadata.session_id, "session-missing-history");
    assert!(snapshot.history.is_empty());
    Ok(())
}

#[test]
fn load_thread_prefers_structured_compaction_event_over_session_counters() -> Result<()> {
    let temp = tempdir()?;
    let rara_dir = temp.path().join(".rara");
    let session_manager = SessionManager::new_for_rara_dir(rara_dir.clone())?;
    let state_db = StateDb::new_for_root_dir(rara_dir)?;
    state_db.upsert_session(
        "session-compaction-event",
        "/tmp/workspace",
        "main",
        "ollama",
        "qwen3",
        None,
        "execute",
        "always",
        None,
        &PersistedPromptRuntimeState::default(),
        0,
        0,
        &PersistedCompactState {
            compaction_count: 1,
            last_compaction_before_tokens: Some(5000),
            last_compaction_after_tokens: Some(1500),
            last_compaction_recent_file_count: Some(0),
            last_compaction_boundary_version: Some(1),
        },
    )?;
    session_manager.save_compaction_event(
        "session-compaction-event",
        &PersistedCompactionEvent {
            event_index: 4,
            before_tokens: 12000,
            after_tokens: 3100,
            boundary_version: 3,
            recent_files: vec!["src/agent/compact.rs".to_string()],
            summary: "Compacted long planning history.".to_string(),
        },
    )?;

    let store = ThreadStore::new(&session_manager, &state_db);
    let snapshot = store.load_thread("session-compaction-event")?;

    assert_eq!(snapshot.compaction.compaction_count, 4);
    assert_eq!(snapshot.compaction.before_tokens, Some(12_000));
    assert_eq!(snapshot.compaction.after_tokens, Some(3_100));
    assert_eq!(
        snapshot.compaction.summary.as_deref(),
        Some("Compacted long planning history.")
    );
    Ok(())
}

#[test]
fn load_thread_prefers_structured_runtime_rollout_items() -> Result<()> {
    let temp = tempdir()?;
    let rara_dir = temp.path().join(".rara");
    let session_manager = SessionManager::new_for_rara_dir(rara_dir.clone())?;
    let state_db = StateDb::new_for_root_dir(rara_dir)?;
    state_db.upsert_session(
        "session-runtime-rollout",
        "/tmp/workspace",
        "main",
        "ollama",
        "qwen3",
        None,
        "execute",
        "always",
        Some("State DB summary should not override runtime rollout."),
        &PersistedPromptRuntimeState::default(),
        0,
        0,
        &PersistedCompactState::default(),
    )?;
    state_db.replace_plan_steps(
        "session-runtime-rollout",
        &[PersistedPlanStep {
            step_index: 0,
            status: "pending".to_string(),
            step: "Legacy side-table plan".to_string(),
        }],
    )?;
    state_db.replace_interactions(
        "session-runtime-rollout",
        &[PersistedInteraction {
            kind: "approval".to_string(),
            status: "pending".to_string(),
            title: "Legacy Approval".to_string(),
            summary: "legacy".to_string(),
            payload: None,
        }],
    )?;
    state_db.replace_runtime_rollout_events(
        "session-runtime-rollout",
        &[
            crate::state_db::PersistedStructuredRolloutEvent::PlanState {
                explanation: Some("Structured rollout plan".to_string()),
                steps: vec![PersistedPlanStep {
                    step_index: 0,
                    status: "in_progress".to_string(),
                    step: "Structured rollout plan step".to_string(),
                }],
            },
            crate::state_db::PersistedStructuredRolloutEvent::Interaction(PersistedInteraction {
                kind: "request_input".to_string(),
                status: "completed".to_string(),
                title: "Structured Question".to_string(),
                summary: "answered".to_string(),
                payload: None,
            }),
        ],
    )?;

    let store = ThreadStore::new(&session_manager, &state_db);
    let snapshot = store.load_thread("session-runtime-rollout")?;

    assert!(matches!(
        snapshot.rollout_items.get(0),
        Some(RolloutItem::PlanState { explanation, steps })
            if explanation.as_deref() == Some("Structured rollout plan")
                && steps[0].step == "Structured rollout plan step"
    ));
    assert!(matches!(
        snapshot.rollout_items.get(1),
        Some(RolloutItem::Interaction(interaction))
            if interaction.title == "Structured Question" && interaction.summary == "answered"
    ));
    Ok(())
}

#[test]
fn thread_recorder_persists_runtime_state_via_state_db() -> Result<()> {
    let temp = tempdir()?;
    let state_db = StateDb::new_for_root_dir(temp.path().join(".rara"))?;
    let recorder = ThreadRecorder::new(&state_db);

    recorder.persist_runtime_state(&ThreadRuntimeState {
        session_id: "session-recorder",
        cwd: "/tmp/workspace",
        branch: "main",
        provider: "ollama",
        model: "qwen3",
        base_url: Some("http://localhost:11434"),
        agent_mode: "execute",
        bash_approval: "always",
        plan_explanation: Some("Keep persistence writes structured."),
        prompt_runtime: PersistedPromptRuntimeState::default(),
        history_len: 3,
        transcript_len: 2,
        compact_state: PersistedCompactState {
            compaction_count: 1,
            last_compaction_before_tokens: Some(4000),
            last_compaction_after_tokens: Some(1200),
            last_compaction_recent_file_count: Some(2),
            last_compaction_boundary_version: Some(3),
        },
    })?;

    let threads = state_db.list_recent_thread_summaries(1)?;
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].session_id, "session-recorder");
    assert_eq!(threads[0].compaction_count, 1);
    assert_eq!(threads[0].last_compaction_after_tokens, Some(1200));
    Ok(())
}

#[test]
fn list_recent_threads_exposes_thread_metadata_surface() -> Result<()> {
    let temp = tempdir()?;
    let state_db = StateDb::new_for_root_dir(temp.path().join(".rara"))?;
    let recorder = ThreadRecorder::new(&state_db);

    recorder.persist_runtime_state(&ThreadRuntimeState {
        session_id: "session-thread-summary",
        cwd: "/tmp/workspace",
        branch: "feature/thread-store",
        provider: "ollama",
        model: "qwen3",
        base_url: None,
        agent_mode: "execute",
        bash_approval: "suggestion",
        plan_explanation: None,
        prompt_runtime: PersistedPromptRuntimeState::default(),
        history_len: 2,
        transcript_len: 1,
        compact_state: PersistedCompactState {
            compaction_count: 2,
            last_compaction_before_tokens: Some(4096),
            last_compaction_after_tokens: Some(1536),
            last_compaction_recent_file_count: Some(1),
            last_compaction_boundary_version: Some(4),
        },
    })?;
    state_db.persist_turn(
        "session-thread-summary",
        0,
        &[PersistedTurnEntry {
            role: "Agent".to_string(),
            message: "Preview line".to_string(),
        }],
    )?;

    let threads = ThreadStore::list_recent_threads_for_db(&state_db, 5)?;

    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].metadata.session_id, "session-thread-summary");
    assert_eq!(threads[0].metadata.branch, "feature/thread-store");
    assert_eq!(threads[0].metadata.cwd, "/tmp/workspace");
    assert_eq!(threads[0].metadata.agent_mode, "execute");
    assert_eq!(threads[0].metadata.bash_approval, "suggestion");
    assert_eq!(threads[0].metadata.history_len, 2);
    assert_eq!(threads[0].metadata.transcript_len, 1);
    assert_eq!(threads[0].preview, "Agent: Preview line");
    assert_eq!(threads[0].compaction.compaction_count, 2);
    assert_eq!(threads[0].compaction.after_tokens, Some(1536));
    Ok(())
}
