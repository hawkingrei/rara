use crate::agent::Message;
use crate::state_db::PersistedStructuredRolloutEvent;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersistedCompactionEvent {
    pub event_index: usize,
    pub before_tokens: usize,
    pub after_tokens: usize,
    pub boundary_version: u32,
    pub recent_files: Vec<String>,
    pub summary: String,
}

pub struct SessionManager {
    pub storage_dir: PathBuf,
    pub legacy_storage_dir: PathBuf,
}

impl SessionManager {
    pub fn is_missing_thread_history_error(err: &anyhow::Error) -> bool {
        err.to_string().contains("Thread not found locally")
    }

    pub fn new() -> Result<Self> {
        let root = std::env::current_dir()?;
        let rara_dir = rara_config::workspace_data_dir_for(&root)?;
        Self::new_for_rara_dir(rara_dir)
    }

    pub fn new_for_rara_dir(rara_dir: PathBuf) -> Result<Self> {
        let local_dir = rara_dir.join("rollouts");
        let legacy_storage_dir = rara_dir.join("sessions");
        if !local_dir.exists() {
            fs::create_dir_all(&local_dir)?;
        }
        if !legacy_storage_dir.exists() {
            fs::create_dir_all(&legacy_storage_dir)?;
        }
        Ok(Self {
            storage_dir: local_dir,
            legacy_storage_dir,
        })
    }

    pub fn save_session(&self, session_id: &str, history: &[Message]) -> Result<()> {
        let path = self.session_history_path(session_id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string(history)?;
        fs::write(path, content)?;
        Ok(())
    }

    pub fn load_thread_history(&self, thread_id: &str) -> Result<Vec<Message>> {
        let path = self.session_history_path(thread_id);
        let content = if path.exists() {
            fs::read_to_string(path)?
        } else {
            let legacy = self.legacy_storage_dir.join(format!("{}.json", thread_id));
            if !legacy.exists() {
                return Err(anyhow!("Thread not found locally"));
            }
            fs::read_to_string(legacy)?
        };
        let history: Vec<Message> = serde_json::from_str(&content)?;
        Ok(history)
    }

    pub fn load_session(&self, session_id: &str) -> Result<Vec<Message>> {
        self.load_thread_history(session_id)
    }

    pub fn save_compaction_event(
        &self,
        session_id: &str,
        event: &PersistedCompactionEvent,
    ) -> Result<()> {
        self.append_rollout_event(
            session_id,
            PersistedStructuredRolloutEvent::Compaction {
                event_index: event.event_index,
                before_tokens: event.before_tokens,
                after_tokens: event.after_tokens,
                boundary_version: event.boundary_version,
                recent_files: event.recent_files.clone(),
                summary: event.summary.clone(),
            },
        )?;
        Ok(())
    }

    pub fn load_compaction_events(&self, session_id: &str) -> Result<Vec<PersistedCompactionEvent>> {
        let rollout_path = self.session_rollout_events_path(session_id);
        if rollout_path.exists() {
            let content = fs::read_to_string(rollout_path)?;
            let events = serde_json::from_str::<Vec<PersistedStructuredRolloutEvent>>(&content)?;
            let compactions = events
                .into_iter()
                .filter_map(|event| match event {
                    PersistedStructuredRolloutEvent::Compaction {
                        event_index,
                        before_tokens,
                        after_tokens,
                        boundary_version,
                        recent_files,
                        summary,
                    } => Some(PersistedCompactionEvent {
                        event_index,
                        before_tokens,
                        after_tokens,
                        boundary_version,
                        recent_files,
                        summary,
                    }),
                    PersistedStructuredRolloutEvent::PlanState { .. }
                    | PersistedStructuredRolloutEvent::Interaction(_) => None,
                })
                .collect::<Vec<_>>();
            if !compactions.is_empty() {
                return Ok(compactions);
            }
        }
        let path = self.session_compaction_events_path(session_id);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let content = fs::read_to_string(path)?;
        Ok(serde_json::from_str(&content)?)
    }

    pub fn get_context(
        &self,
        session_id: &str,
        turn_index: usize,
        window: usize,
    ) -> Result<Vec<Message>> {
        let history = self.load_thread_history(session_id)?;
        let start = turn_index.saturating_sub(window);
        let end = (turn_index + window + 1).min(history.len());
        Ok(history[start..end].to_vec())
    }

    fn session_history_path(&self, session_id: &str) -> PathBuf {
        self.storage_dir.join(session_id).join("history.json")
    }

    fn session_compaction_events_path(&self, session_id: &str) -> PathBuf {
        self.storage_dir.join(session_id).join("compactions.json")
    }

    fn session_rollout_events_path(&self, session_id: &str) -> PathBuf {
        self.storage_dir.join(session_id).join("events.json")
    }

    fn append_rollout_event(
        &self,
        session_id: &str,
        event: PersistedStructuredRolloutEvent,
    ) -> Result<()> {
        let path = self.session_rollout_events_path(session_id);
        let mut events = if path.exists() {
            let content = fs::read_to_string(&path)?;
            serde_json::from_str::<Vec<PersistedStructuredRolloutEvent>>(&content)?
        } else {
            Vec::new()
        };
        events.push(event);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(&events)?;
        fs::write(path, content)?;
        Ok(())
    }
}
