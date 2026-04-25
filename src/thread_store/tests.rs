use anyhow::Result;
use std::fs;
use tempfile::tempdir;

use crate::agent::Message;
use crate::session::{PersistedCompactionEvent, SessionManager};
use crate::state_db::{
    PersistedCompactState, PersistedInteraction, PersistedPlanStep, PersistedPromptRuntimeState,
    PersistedRuntimeRolloutItem, PersistedStructuredRolloutEvent, PersistedTurnEntry, StateDb,
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
    assert_eq!(
        snapshot.plan_explanation.as_deref(),
        Some("Inspect current plan state.")
    );
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
fn load_thread_backfills_legacy_history_file_into_rollout_root() -> Result<()> {
    let temp = tempdir()?;
    let rara_dir = temp.path().join(".rara");
    let session_manager = SessionManager::new_for_rara_dir(rara_dir.clone())?;
    let state_db = StateDb::new_for_root_dir(rara_dir)?;
    state_db.upsert_session(
        "session-legacy-history",
        "/tmp/workspace",
        "main",
        "ollama",
        "qwen3",
        None,
        "execute",
        "always",
        None,
        &PersistedPromptRuntimeState::default(),
        1,
        0,
        &PersistedCompactState::default(),
    )?;
    fs::write(
        session_manager
            .legacy_storage_dir
            .join("session-legacy-history.json"),
        serde_json::to_string(&vec![Message {
            role: "user".to_string(),
            content: serde_json::json!("legacy thread history"),
        }])?,
    )?;

    let store = ThreadStore::new(&session_manager, &state_db);
    let snapshot = store.load_thread("session-legacy-history")?;

    assert_eq!(snapshot.history.len(), 1);
    let canonical_history = fs::read_to_string(
        session_manager
            .storage_dir
            .join("session-legacy-history")
            .join("history.json"),
    )?;
    let canonical_messages: Vec<Message> = serde_json::from_str(&canonical_history)?;
    assert_eq!(canonical_messages.len(), 1);
    assert_eq!(canonical_messages[0].role, "user");
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
                recorded_at: None,
                explanation: Some("Structured rollout plan".to_string()),
                steps: vec![PersistedPlanStep {
                    step_index: 0,
                    status: "in_progress".to_string(),
                    step: "Structured rollout plan step".to_string(),
                }],
            },
            crate::state_db::PersistedStructuredRolloutEvent::Interaction {
                recorded_at: None,
                interaction: PersistedInteraction {
                    kind: "request_input".to_string(),
                    status: "completed".to_string(),
                    title: "Structured Question".to_string(),
                    summary: "answered".to_string(),
                    payload: None,
                },
            },
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
fn load_thread_falls_back_to_legacy_runtime_rollout_file() -> Result<()> {
    let temp = tempdir()?;
    let rara_dir = temp.path().join(".rara");
    let session_manager = SessionManager::new_for_rara_dir(rara_dir.clone())?;
    let state_db = StateDb::new_for_root_dir(rara_dir)?;
    state_db.upsert_session(
        "session-legacy-runtime-rollout",
        "/tmp/workspace",
        "main",
        "ollama",
        "qwen3",
        None,
        "execute",
        "always",
        Some("State DB summary should not override legacy runtime rollout."),
        &PersistedPromptRuntimeState::default(),
        0,
        0,
        &PersistedCompactState::default(),
    )?;
    state_db.replace_plan_steps(
        "session-legacy-runtime-rollout",
        &[PersistedPlanStep {
            step_index: 0,
            status: "pending".to_string(),
            step: "Legacy side-table plan".to_string(),
        }],
    )?;
    state_db.replace_interactions(
        "session-legacy-runtime-rollout",
        &[PersistedInteraction {
            kind: "approval".to_string(),
            status: "pending".to_string(),
            title: "Legacy Approval".to_string(),
            summary: "legacy".to_string(),
            payload: None,
        }],
    )?;

    let runtime_path = state_db
        .rollout_root()
        .join("session-legacy-runtime-rollout")
        .join("runtime.json");
    fs::create_dir_all(runtime_path.parent().expect("runtime rollout dir"))?;
    fs::write(
        &runtime_path,
        serde_json::to_string_pretty(&vec![
            PersistedRuntimeRolloutItem::PlanState {
                explanation: Some("Legacy runtime rollout plan".to_string()),
                steps: vec![PersistedPlanStep {
                    step_index: 0,
                    status: "in_progress".to_string(),
                    step: "Inspect legacy runtime rollout".to_string(),
                }],
            },
            PersistedRuntimeRolloutItem::Interaction(PersistedInteraction {
                kind: "request_input".to_string(),
                status: "completed".to_string(),
                title: "Legacy Runtime Question".to_string(),
                summary: "answered".to_string(),
                payload: None,
            }),
        ])?,
    )?;

    let store = ThreadStore::new(&session_manager, &state_db);
    let snapshot = store.load_thread("session-legacy-runtime-rollout")?;

    assert!(matches!(
        snapshot.rollout_items.get(0),
        Some(RolloutItem::PlanState { explanation, steps })
            if explanation.as_deref() == Some("Legacy runtime rollout plan")
                && steps[0].step == "Inspect legacy runtime rollout"
    ));
    assert!(matches!(
        snapshot.rollout_items.get(1),
        Some(RolloutItem::Interaction(interaction))
            if interaction.title == "Legacy Runtime Question" && interaction.summary == "answered"
    ));
    Ok(())
}

#[test]
fn load_thread_backfills_legacy_non_turn_rollout_files_into_event_log() -> Result<()> {
    let temp = tempdir()?;
    let rara_dir = temp.path().join(".rara");
    let session_manager = SessionManager::new_for_rara_dir(rara_dir.clone())?;
    let state_db = StateDb::new_for_root_dir(rara_dir)?;
    state_db.upsert_session(
        "session-legacy-non-turn",
        "/tmp/workspace",
        "main",
        "ollama",
        "qwen3",
        None,
        "execute",
        "always",
        Some("State DB summary should not override migrated rollout."),
        &PersistedPromptRuntimeState::default(),
        0,
        0,
        &PersistedCompactState::default(),
    )?;

    let rollout_dir = state_db.rollout_root().join("session-legacy-non-turn");
    fs::create_dir_all(&rollout_dir)?;
    fs::write(
        rollout_dir.join("runtime.json"),
        serde_json::to_string_pretty(&vec![
            PersistedRuntimeRolloutItem::PlanState {
                explanation: Some("Legacy runtime rollout plan".to_string()),
                steps: vec![PersistedPlanStep {
                    step_index: 0,
                    status: "in_progress".to_string(),
                    step: "Inspect migrated rollout".to_string(),
                }],
            },
            PersistedRuntimeRolloutItem::Interaction(PersistedInteraction {
                kind: "request_input".to_string(),
                status: "completed".to_string(),
                title: "Legacy Runtime Question".to_string(),
                summary: "answered".to_string(),
                payload: None,
            }),
        ])?,
    )?;

    let legacy_compactions = session_manager
        .storage_dir
        .join("session-legacy-non-turn")
        .join("compactions.json");
    fs::create_dir_all(legacy_compactions.parent().expect("legacy compaction dir"))?;
    fs::write(
        &legacy_compactions,
        serde_json::to_string_pretty(&vec![PersistedCompactionEvent {
            event_index: 1,
            before_tokens: 1200,
            after_tokens: 320,
            boundary_version: 2,
            recent_files: vec!["src/thread_store.rs".to_string()],
            summary: "Legacy compaction".to_string(),
        }])?,
    )?;

    let store = ThreadStore::new(&session_manager, &state_db);
    let snapshot = store.load_thread("session-legacy-non-turn")?;

    assert_eq!(
        snapshot.plan_explanation.as_deref(),
        Some("Legacy runtime rollout plan")
    );
    assert_eq!(snapshot.interactions.len(), 1);
    assert_eq!(snapshot.compaction.compaction_count, 1);

    let rollout_log = fs::read_to_string(rollout_dir.join("events.jsonl"))?;
    let rollout_events = rollout_log
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(serde_json::from_str::<PersistedStructuredRolloutEvent>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    assert!(rollout_events.iter().any(|event| matches!(
        event,
        PersistedStructuredRolloutEvent::PlanState {
            recorded_at: _,
            explanation,
            ..
        }
            if explanation.as_deref() == Some("Legacy runtime rollout plan")
    )));
    assert!(rollout_events.iter().any(|event| matches!(
        event,
        PersistedStructuredRolloutEvent::Interaction {
            recorded_at: _,
            interaction,
        }
            if interaction.title == "Legacy Runtime Question"
    )));
    assert!(rollout_events.iter().any(|event| matches!(
        event,
        PersistedStructuredRolloutEvent::Compaction {
            event_index,
            summary,
            ..
        } if *event_index == 1 && summary == "Legacy compaction"
    )));
    Ok(())
}

#[test]
fn load_thread_preserves_structured_rollout_event_order() -> Result<()> {
    let temp = tempdir()?;
    let rara_dir = temp.path().join(".rara");
    let session_manager = SessionManager::new_for_rara_dir(rara_dir.clone())?;
    let state_db = StateDb::new_for_root_dir(rara_dir)?;
    state_db.upsert_session(
        "session-ordered-events",
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
    session_manager.save_compaction_event(
        "session-ordered-events",
        &PersistedCompactionEvent {
            event_index: 1,
            before_tokens: 9000,
            after_tokens: 3200,
            boundary_version: 2,
            recent_files: vec!["src/agent/compact.rs".to_string()],
            summary: "Compacted repository scan.".to_string(),
        },
    )?;
    state_db.replace_runtime_rollout_events(
        "session-ordered-events",
        &[
            crate::state_db::PersistedStructuredRolloutEvent::PlanState {
                recorded_at: None,
                explanation: Some("Plan after first compaction".to_string()),
                steps: vec![PersistedPlanStep {
                    step_index: 0,
                    status: "in_progress".to_string(),
                    step: "Inspect runtime events".to_string(),
                }],
            },
            crate::state_db::PersistedStructuredRolloutEvent::Interaction {
                recorded_at: None,
                interaction: PersistedInteraction {
                    kind: "request_input".to_string(),
                    status: "pending".to_string(),
                    title: "Question".to_string(),
                    summary: "Need confirmation".to_string(),
                    payload: None,
                },
            },
        ],
    )?;
    session_manager.save_compaction_event(
        "session-ordered-events",
        &PersistedCompactionEvent {
            event_index: 2,
            before_tokens: 6000,
            after_tokens: 2500,
            boundary_version: 3,
            recent_files: vec!["src/thread_store.rs".to_string()],
            summary: "Compacted after planning.".to_string(),
        },
    )?;

    let store = ThreadStore::new(&session_manager, &state_db);
    let snapshot = store.load_thread("session-ordered-events")?;

    assert!(matches!(
        snapshot.rollout_items.get(0),
        Some(RolloutItem::Compaction(compaction))
            if compaction.compaction_count == 1
    ));
    assert!(matches!(
        snapshot.rollout_items.get(1),
        Some(RolloutItem::PlanState { explanation, .. })
            if explanation.as_deref() == Some("Plan after first compaction")
    ));
    assert!(matches!(
        snapshot.rollout_items.get(2),
        Some(RolloutItem::Interaction(interaction))
            if interaction.title == "Question"
    ));
    assert!(matches!(
        snapshot.rollout_items.get(3),
        Some(RolloutItem::Compaction(compaction))
            if compaction.compaction_count == 2
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

#[test]
fn fork_thread_preserves_materialized_state_and_sets_lineage() -> Result<()> {
    let temp = tempdir()?;
    let rara_dir = temp.path().join(".rara");
    let session_manager = SessionManager::new_for_rara_dir(rara_dir.clone())?;
    let state_db = StateDb::new_for_root_dir(rara_dir)?;
    session_manager.save_session(
        "source-thread",
        &[Message {
            role: "user".to_string(),
            content: serde_json::json!("continue this implementation"),
        }],
    )?;
    state_db.upsert_session(
        "source-thread",
        "/tmp/workspace",
        "main",
        "codex",
        "gpt-5",
        Some("https://chatgpt.com/backend-api/codex"),
        "execute",
        "on-request",
        Some("Preserve thread continuity."),
        &PersistedPromptRuntimeState {
            append_system_prompt: Some("Keep the fork aligned with the parent thread.".to_string()),
            warnings: vec!["using local thread store".to_string()],
        },
        1,
        1,
        &PersistedCompactState {
            compaction_count: 2,
            last_compaction_before_tokens: Some(9000),
            last_compaction_after_tokens: Some(2400),
            last_compaction_recent_file_count: Some(1),
            last_compaction_boundary_version: Some(3),
        },
    )?;
    state_db.replace_plan_steps(
        "source-thread",
        &[PersistedPlanStep {
            step_index: 0,
            status: "in_progress".to_string(),
            step: "Implement fork lifecycle".to_string(),
        }],
    )?;
    state_db.replace_interactions(
        "source-thread",
        &[PersistedInteraction {
            kind: "approval".to_string(),
            status: "completed".to_string(),
            title: "Approved".to_string(),
            summary: "continue".to_string(),
            payload: None,
        }],
    )?;
    state_db.persist_turn(
        "source-thread",
        0,
        &[PersistedTurnEntry {
            role: "Agent".to_string(),
            message: "Implementing the fork command.".to_string(),
        }],
    )?;
    session_manager.save_compaction_event(
        "source-thread",
        &PersistedCompactionEvent {
            event_index: 2,
            before_tokens: 9000,
            after_tokens: 2400,
            boundary_version: 3,
            recent_files: vec!["src/thread_store.rs".to_string()],
            summary: "Compacted earlier lifecycle exploration.".to_string(),
        },
    )?;

    let store = ThreadStore::new(&session_manager, &state_db);
    let forked_thread_id = store.fork_thread("source-thread")?;
    assert_ne!(forked_thread_id, "source-thread");

    let snapshot = store.load_thread(&forked_thread_id)?;
    assert_eq!(snapshot.metadata.origin_kind, "fork");
    assert_eq!(
        snapshot.metadata.forked_from_thread_id.as_deref(),
        Some("source-thread")
    );
    assert_eq!(snapshot.history.len(), 1);
    assert_eq!(
        snapshot.plan_explanation.as_deref(),
        Some("Preserve thread continuity.")
    );
    assert_eq!(snapshot.plan_steps.len(), 1);
    assert_eq!(snapshot.interactions.len(), 1);
    assert_eq!(snapshot.compaction.compaction_count, 2);
    assert!(matches!(
        snapshot.rollout_items.last(),
        Some(RolloutItem::Turn(turn))
            if turn.entries[0].message == "Implementing the fork command."
    ));

    let runtime_state = state_db
        .load_session_runtime_state(&forked_thread_id)?
        .expect("forked runtime state");
    assert_eq!(
        runtime_state.prompt_runtime.append_system_prompt.as_deref(),
        Some("Keep the fork aligned with the parent thread.")
    );
    assert_eq!(
        runtime_state.prompt_runtime.warnings,
        vec!["using local thread store".to_string()]
    );

    let rollout_events = state_db.load_rollout_events(&forked_thread_id)?;
    assert!(rollout_events.iter().any(|event| matches!(
        event,
        PersistedStructuredRolloutEvent::Compaction {
            event_index,
            summary,
            ..
        } if *event_index == 2 && summary == "Compacted earlier lifecycle exploration."
    )));
    assert!(rollout_events.iter().any(|event| matches!(
        event,
        PersistedStructuredRolloutEvent::RuntimeState {
            recorded_at: _,
            explanation,
            steps,
            interactions,
        }
            if explanation.as_deref() == Some("Preserve thread continuity.")
                && steps.len() == 1
                && interactions.len() == 1
    )));

    Ok(())
}
