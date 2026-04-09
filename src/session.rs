use std::fs;
use std::path::{PathBuf};
use anyhow::{Result, anyhow};
use crate::agent::Message;

pub struct SessionManager {
    pub storage_dir: PathBuf,
}

impl SessionManager {
    pub fn new() -> Result<Self> {
        let local_dir = std::env::current_dir()?.join(".rara/sessions");
        if !local_dir.exists() {
            fs::create_dir_all(&local_dir)?;
        }
        Ok(Self { storage_dir: local_dir })
    }

    pub fn save_session(&self, session_id: &str, history: &[Message]) -> Result<()> {
        let path = self.storage_dir.join(format!("{}.json", session_id));
        let content = serde_json::to_string(history)?;
        fs::write(path, content)?;
        Ok(())
    }

    pub fn load_session(&self, session_id: &str) -> Result<Vec<Message>> {
        let path = self.storage_dir.join(format!("{}.json", session_id));
        if !path.exists() {
            return Err(anyhow!("Session not found locally"));
        }
        let content = fs::read_to_string(path)?;
        let history: Vec<Message> = serde_json::from_str(&content)?;
        Ok(history)
    }

    pub fn get_context(&self, session_id: &str, turn_index: usize, window: usize) -> Result<Vec<Message>> {
        let history = self.load_session(session_id)?;
        let start = turn_index.saturating_sub(window);
        let end = (turn_index + window + 1).min(history.len());
        Ok(history[start..end].to_vec())
    }
}
