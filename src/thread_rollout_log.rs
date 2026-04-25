use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use anyhow::Result;

use crate::state_db::PersistedStructuredRolloutEvent;

fn rollout_log_write_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub(crate) fn rollout_events_log_path(root_dir: &Path, thread_id: &str) -> PathBuf {
    root_dir.join(thread_id).join("events.jsonl")
}

pub(crate) fn rollout_events_snapshot_path(root_dir: &Path, thread_id: &str) -> PathBuf {
    root_dir.join(thread_id).join("events.json")
}

pub(crate) fn append_rollout_event_line(
    root_dir: &Path,
    thread_id: &str,
    event: &PersistedStructuredRolloutEvent,
) -> Result<()> {
    let _guard = rollout_log_write_lock()
        .lock()
        .expect("rollout log write mutex poisoned");
    let path = rollout_events_log_path(root_dir, thread_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    serde_json::to_writer(&mut file, event)?;
    file.write_all(b"\n")?;
    Ok(())
}

pub(crate) fn load_rollout_events(
    root_dir: &Path,
    thread_id: &str,
) -> Result<Vec<PersistedStructuredRolloutEvent>> {
    let mut events = Vec::new();

    let append_only_path = rollout_events_log_path(root_dir, thread_id);
    if append_only_path.exists() {
        let content = fs::read_to_string(append_only_path)?;
        let append_only_events = content
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(serde_json::from_str)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        events.extend(append_only_events);
    }

    let snapshot_path = rollout_events_snapshot_path(root_dir, thread_id);
    if snapshot_path.exists() {
        let content = fs::read_to_string(snapshot_path)?;
        let snapshot_events =
            serde_json::from_str::<Vec<PersistedStructuredRolloutEvent>>(&content)?;
        events.extend(snapshot_events);
    }

    Ok(events)
}
