use crate::agent::Message;
use anyhow::{anyhow, Result};
use std::fs;
use std::path::PathBuf;

pub struct SessionManager {
    pub storage_dir: PathBuf,
    pub legacy_storage_dir: PathBuf,
}

impl SessionManager {
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

    pub fn load_session(&self, session_id: &str) -> Result<Vec<Message>> {
        let path = self.session_history_path(session_id);
        let content = if path.exists() {
            fs::read_to_string(path)?
        } else {
            let legacy = self.legacy_storage_dir.join(format!("{}.json", session_id));
            if !legacy.exists() {
                return Err(anyhow!("Session not found locally"));
            }
            fs::read_to_string(legacy)?
        };
        let history: Vec<Message> = serde_json::from_str(&content)?;
        Ok(history)
    }

    pub fn get_context(
        &self,
        session_id: &str,
        turn_index: usize,
        window: usize,
    ) -> Result<Vec<Message>> {
        let history = self.load_session(session_id)?;
        let start = turn_index.saturating_sub(window);
        let end = (turn_index + window + 1).min(history.len());
        Ok(history[start..end].to_vec())
    }

    fn session_history_path(&self, session_id: &str) -> PathBuf {
        self.storage_dir.join(session_id).join("history.json")
    }
}
