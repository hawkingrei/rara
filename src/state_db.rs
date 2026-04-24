use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedTurnEntry {
    pub role: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedPlanStep {
    pub step_index: usize,
    pub status: String,
    pub step: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedInteraction {
    pub kind: String,
    pub status: String,
    pub title: String,
    pub summary: String,
    pub payload: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct PersistedTurnSummary {
    pub ordinal: usize,
    pub event_count: usize,
    pub artifact_path: String,
    pub preview: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PersistedRuntimeRolloutItem {
    PlanState {
        explanation: Option<String>,
        steps: Vec<PersistedPlanStep>,
    },
    Interaction(PersistedInteraction),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PersistedStructuredRolloutEvent {
    Compaction {
        event_index: usize,
        before_tokens: usize,
        after_tokens: usize,
        boundary_version: u32,
        recent_files: Vec<String>,
        summary: String,
    },
    PlanState {
        explanation: Option<String>,
        steps: Vec<PersistedPlanStep>,
    },
    Interaction(PersistedInteraction),
}

#[derive(Debug, Clone)]
pub struct PersistedRecentThreadSummary {
    pub session_id: String,
    pub provider: String,
    pub model: String,
    pub branch: String,
    pub updated_at: i64,
    pub preview: String,
    pub compaction_count: usize,
    pub last_compaction_before_tokens: Option<usize>,
    pub last_compaction_after_tokens: Option<usize>,
    pub last_compaction_recent_file_count: Option<usize>,
    pub last_compaction_boundary_version: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct PersistedRecentThreadRecord {
    pub session_id: String,
    pub cwd: String,
    pub branch: String,
    pub provider: String,
    pub model: String,
    pub base_url: Option<String>,
    pub agent_mode: String,
    pub bash_approval: String,
    pub history_len: usize,
    pub transcript_len: usize,
    pub updated_at: i64,
    pub preview: String,
    pub compaction_count: usize,
    pub last_compaction_before_tokens: Option<usize>,
    pub last_compaction_after_tokens: Option<usize>,
    pub last_compaction_recent_file_count: Option<usize>,
    pub last_compaction_boundary_version: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct PersistedThreadRecord {
    pub session_id: String,
    pub cwd: String,
    pub branch: String,
    pub provider: String,
    pub model: String,
    pub base_url: Option<String>,
    pub agent_mode: String,
    pub bash_approval: String,
    pub plan_explanation: Option<String>,
    pub history_len: usize,
    pub transcript_len: usize,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Default)]
pub struct PersistedCompactState {
    pub compaction_count: usize,
    pub last_compaction_before_tokens: Option<usize>,
    pub last_compaction_after_tokens: Option<usize>,
    pub last_compaction_recent_file_count: Option<usize>,
    pub last_compaction_boundary_version: Option<u32>,
}

pub struct StateDb {
    root_dir: PathBuf,
    path: PathBuf,
    conn: Mutex<Connection>,
}

impl StateDb {
    pub fn new() -> Result<Self> {
        let root = std::env::current_dir()?;
        let root_dir = rara_config::workspace_data_dir_for(&root)?;
        Self::new_for_root_dir(root_dir)
    }

    pub fn new_for_root_dir(root_dir: PathBuf) -> Result<Self> {
        if !root_dir.exists() {
            fs::create_dir_all(&root_dir)?;
        }
        let rollout_dir = root_dir.join("rollouts");
        if !rollout_dir.exists() {
            fs::create_dir_all(&rollout_dir)?;
        }
        let path = root_dir.join("state.sqlite3");
        let conn = Connection::open(&path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        let db = Self {
            root_dir,
            path,
            conn: Mutex::new(conn),
        };
        db.init_schema()?;
        Ok(db)
    }

    pub fn path(&self) -> &Path {
        self.path.as_path()
    }

    pub fn rollout_root(&self) -> PathBuf {
        self.root_dir.join("rollouts")
    }

    pub fn upsert_session(
        &self,
        session_id: &str,
        cwd: &str,
        branch: &str,
        provider: &str,
        model: &str,
        base_url: Option<&str>,
        agent_mode: &str,
        bash_approval: &str,
        plan_explanation: Option<&str>,
        history_len: usize,
        transcript_len: usize,
        compact_state: &PersistedCompactState,
    ) -> Result<()> {
        let now = epoch_seconds();
        let conn = self.conn.lock().expect("state db mutex poisoned");
        conn.execute(
            "INSERT INTO sessions (
                id, cwd, branch, provider, model, base_url, agent_mode, bash_approval,
                plan_explanation, history_len, transcript_len, compaction_count,
                last_compaction_before_tokens, last_compaction_after_tokens,
                last_compaction_recent_file_count, last_compaction_boundary_version,
                created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                cwd = excluded.cwd,
                branch = excluded.branch,
                provider = excluded.provider,
                model = excluded.model,
                base_url = excluded.base_url,
                agent_mode = excluded.agent_mode,
                bash_approval = excluded.bash_approval,
                plan_explanation = excluded.plan_explanation,
                history_len = excluded.history_len,
                transcript_len = excluded.transcript_len,
                compaction_count = excluded.compaction_count,
                last_compaction_before_tokens = excluded.last_compaction_before_tokens,
                last_compaction_after_tokens = excluded.last_compaction_after_tokens,
                last_compaction_recent_file_count = excluded.last_compaction_recent_file_count,
                last_compaction_boundary_version = excluded.last_compaction_boundary_version,
                updated_at = excluded.updated_at",
            params![
                session_id,
                cwd,
                branch,
                provider,
                model,
                base_url,
                agent_mode,
                bash_approval,
                plan_explanation,
                history_len as i64,
                transcript_len as i64,
                compact_state.compaction_count as i64,
                compact_state
                    .last_compaction_before_tokens
                    .map(|value| value as i64),
                compact_state
                    .last_compaction_after_tokens
                    .map(|value| value as i64),
                compact_state
                    .last_compaction_recent_file_count
                    .map(|value| value as i64),
                compact_state
                    .last_compaction_boundary_version
                    .map(|value| value as i64),
                now,
                now
            ],
        )?;
        Ok(())
    }

    pub fn replace_plan_steps(&self, session_id: &str, steps: &[PersistedPlanStep]) -> Result<()> {
        let mut conn = self.conn.lock().expect("state db mutex poisoned");
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM plan_steps WHERE session_id = ?",
            params![session_id],
        )?;
        for step in steps {
            tx.execute(
                "INSERT INTO plan_steps (session_id, step_index, status, step, updated_at)
                 VALUES (?, ?, ?, ?, ?)",
                params![
                    session_id,
                    step.step_index as i64,
                    step.status,
                    step.step,
                    epoch_seconds()
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn replace_interactions(
        &self,
        session_id: &str,
        interactions: &[PersistedInteraction],
    ) -> Result<()> {
        let mut conn = self.conn.lock().expect("state db mutex poisoned");
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM interactions WHERE session_id = ?",
            params![session_id],
        )?;
        for interaction in interactions {
            tx.execute(
                "INSERT INTO interactions (session_id, kind, status, title, summary, payload_json, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
                params![
                    session_id,
                    interaction.kind,
                    interaction.status,
                    interaction.title,
                    interaction.summary,
                    interaction
                        .payload
                        .as_ref()
                        .map(serde_json::to_string)
                        .transpose()?,
                    epoch_seconds()
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn persist_turn(
        &self,
        session_id: &str,
        ordinal: usize,
        entries: &[PersistedTurnEntry],
    ) -> Result<PersistedTurnSummary> {
        let artifact_path = self.write_turn_artifact(session_id, ordinal, entries)?;
        let preview = turn_preview(entries);
        let now = epoch_seconds();
        let conn = self.conn.lock().expect("state db mutex poisoned");
        conn.execute(
            "INSERT INTO turns (
                session_id, ordinal, event_count, artifact_path, preview, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(session_id, ordinal) DO UPDATE SET
                event_count = excluded.event_count,
                artifact_path = excluded.artifact_path,
                preview = excluded.preview,
                updated_at = excluded.updated_at",
            params![
                session_id,
                ordinal as i64,
                entries.len() as i64,
                artifact_path,
                preview,
                now,
                now
            ],
        )?;
        Ok(PersistedTurnSummary {
            ordinal,
            event_count: entries.len(),
            artifact_path: self
                .artifact_relative_path(session_id, ordinal)
                .display()
                .to_string(),
            preview: turn_preview(entries),
        })
    }

    pub fn load_turn_entries(
        &self,
        session_id: &str,
        ordinal: usize,
    ) -> Result<Vec<PersistedTurnEntry>> {
        let path = self
            .rollout_root()
            .join(self.artifact_relative_path(session_id, ordinal));
        if !path.exists() {
            return Ok(Vec::new());
        }
        let content = fs::read_to_string(path)?;
        Ok(serde_json::from_str(&content)?)
    }

    pub fn load_turn_summaries(&self, session_id: &str) -> Result<Vec<PersistedTurnSummary>> {
        let conn = self.conn.lock().expect("state db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT ordinal, event_count, artifact_path, preview
             FROM turns
             WHERE session_id = ?
             ORDER BY ordinal ASC",
        )?;
        let rows = stmt.query_map(params![session_id], |row| {
            Ok(PersistedTurnSummary {
                ordinal: row.get::<_, i64>(0)? as usize,
                event_count: row.get::<_, i64>(1)? as usize,
                artifact_path: row.get(2)?,
                preview: row.get(3)?,
            })
        })?;
        let mut summaries = Vec::new();
        for row in rows {
            summaries.push(row?);
        }
        Ok(summaries)
    }

    pub fn replace_runtime_rollout(
        &self,
        session_id: &str,
        items: &[PersistedRuntimeRolloutItem],
    ) -> Result<()> {
        let path = self.runtime_rollout_path(session_id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(items)?;
        fs::write(path, content)?;
        Ok(())
    }

    pub fn load_runtime_rollout(&self, session_id: &str) -> Result<Vec<PersistedRuntimeRolloutItem>> {
        let path = self.runtime_rollout_path(session_id);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let content = fs::read_to_string(path)?;
        Ok(serde_json::from_str(&content)?)
    }

    pub fn load_rollout_events(
        &self,
        session_id: &str,
    ) -> Result<Vec<PersistedStructuredRolloutEvent>> {
        let path = self.rollout_events_path(session_id);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let content = fs::read_to_string(path)?;
        Ok(serde_json::from_str(&content)?)
    }

    pub fn append_compaction_rollout_event(
        &self,
        session_id: &str,
        event_index: usize,
        before_tokens: usize,
        after_tokens: usize,
        boundary_version: u32,
        recent_files: &[String],
        summary: &str,
    ) -> Result<()> {
        let mut events = self.load_rollout_events(session_id)?;
        events.push(PersistedStructuredRolloutEvent::Compaction {
            event_index,
            before_tokens,
            after_tokens,
            boundary_version,
            recent_files: recent_files.to_vec(),
            summary: summary.to_string(),
        });
        self.write_rollout_events(session_id, &events)
    }

    pub fn replace_runtime_rollout_events(
        &self,
        session_id: &str,
        items: &[PersistedStructuredRolloutEvent],
    ) -> Result<()> {
        let mut preserved = self
            .load_rollout_events(session_id)?
            .into_iter()
            .filter(|item| matches!(item, PersistedStructuredRolloutEvent::Compaction { .. }))
            .collect::<Vec<_>>();
        preserved.extend_from_slice(items);
        self.write_rollout_events(session_id, &preserved)
    }

    pub fn load_plan_steps(&self, session_id: &str) -> Result<Vec<PersistedPlanStep>> {
        let conn = self.conn.lock().expect("state db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT step_index, status, step
             FROM plan_steps
             WHERE session_id = ?
             ORDER BY step_index ASC",
        )?;
        let rows = stmt.query_map(params![session_id], |row| {
            Ok(PersistedPlanStep {
                step_index: row.get::<_, i64>(0)? as usize,
                status: row.get(1)?,
                step: row.get(2)?,
            })
        })?;
        let mut steps = Vec::new();
        for row in rows {
            steps.push(row?);
        }
        Ok(steps)
    }

    pub fn load_interactions(&self, session_id: &str) -> Result<Vec<PersistedInteraction>> {
        let conn = self.conn.lock().expect("state db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT kind, status, title, summary, payload_json
             FROM interactions
             WHERE session_id = ?
             ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(params![session_id], |row| {
            let payload_json: Option<String> = row.get(4)?;
            Ok(PersistedInteraction {
                kind: row.get(0)?,
                status: row.get(1)?,
                title: row.get(2)?,
                summary: row.get(3)?,
                payload: payload_json
                    .as_deref()
                    .filter(|value| !value.is_empty())
                    .map(serde_json::from_str)
                    .transpose()
                    .map_err(|err| {
                        rusqlite::Error::FromSqlConversionFailure(
                            4,
                            rusqlite::types::Type::Text,
                            Box::new(err),
                        )
                    })?,
            })
        })?;
        let mut interactions = Vec::new();
        for row in rows {
            interactions.push(row?);
        }
        Ok(interactions)
    }

    pub fn load_session_plan_explanation(&self, session_id: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().expect("state db mutex poisoned");
        let explanation = conn.query_row(
            "SELECT plan_explanation FROM sessions WHERE id = ?",
            params![session_id],
            |row| row.get::<_, Option<String>>(0),
        )?;
        Ok(explanation)
    }

    pub fn load_session_compact_state(&self, session_id: &str) -> Result<PersistedCompactState> {
        let conn = self.conn.lock().expect("state db mutex poisoned");
        let compact_state = conn.query_row(
            "SELECT compaction_count, last_compaction_before_tokens,
                    last_compaction_after_tokens, last_compaction_recent_file_count,
                    last_compaction_boundary_version
             FROM sessions
             WHERE id = ?",
            params![session_id],
            |row| {
                Ok(PersistedCompactState {
                    compaction_count: row.get::<_, i64>(0)? as usize,
                    last_compaction_before_tokens: row
                        .get::<_, Option<i64>>(1)?
                        .map(|value| value as usize),
                    last_compaction_after_tokens: row
                        .get::<_, Option<i64>>(2)?
                        .map(|value| value as usize),
                    last_compaction_recent_file_count: row
                        .get::<_, Option<i64>>(3)?
                        .map(|value| value as usize),
                    last_compaction_boundary_version: row
                        .get::<_, Option<i64>>(4)?
                        .map(|value| value as u32),
                })
            },
        );
        match compact_state {
            Ok(compact_state) => Ok(compact_state),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(PersistedCompactState::default()),
            Err(err) => Err(err.into()),
        }
    }

    pub fn latest_thread_id(&self) -> Result<Option<String>> {
        let conn = self.conn.lock().expect("state db mutex poisoned");
        let thread_id = conn.query_row(
            "SELECT id FROM sessions ORDER BY updated_at DESC LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        );
        match thread_id {
            Ok(id) => Ok(Some(id)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    pub fn load_thread_record(&self, session_id: &str) -> Result<Option<PersistedThreadRecord>> {
        let conn = self.conn.lock().expect("state db mutex poisoned");
        let record = conn.query_row(
            "SELECT id, cwd, branch, provider, model, base_url, agent_mode, bash_approval,
                    plan_explanation, history_len, transcript_len, updated_at
             FROM sessions
             WHERE id = ?",
            params![session_id],
            |row| {
                Ok(PersistedThreadRecord {
                    session_id: row.get(0)?,
                    cwd: row.get(1)?,
                    branch: row.get(2)?,
                    provider: row.get(3)?,
                    model: row.get(4)?,
                    base_url: row.get(5)?,
                    agent_mode: row.get(6)?,
                    bash_approval: row.get(7)?,
                    plan_explanation: row.get(8)?,
                    history_len: row.get::<_, i64>(9)? as usize,
                    transcript_len: row.get::<_, i64>(10)? as usize,
                    updated_at: row.get(11)?,
                })
            },
        );
        match record {
            Ok(record) => Ok(Some(record)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    pub fn list_recent_thread_summaries(
        &self,
        limit: usize,
    ) -> Result<Vec<PersistedRecentThreadSummary>> {
        let conn = self.conn.lock().expect("state db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT s.id, s.provider, s.model, s.branch, s.updated_at,
                    s.compaction_count, s.last_compaction_before_tokens,
                    s.last_compaction_after_tokens, s.last_compaction_recent_file_count,
                    s.last_compaction_boundary_version,
                    COALESCE((
                        SELECT preview FROM turns
                        WHERE session_id = s.id
                        ORDER BY ordinal DESC
                        LIMIT 1
                    ), '') AS preview
             FROM sessions s
             ORDER BY s.updated_at DESC
             LIMIT ?",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(PersistedRecentThreadSummary {
                session_id: row.get(0)?,
                provider: row.get(1)?,
                model: row.get(2)?,
                branch: row.get(3)?,
                updated_at: row.get(4)?,
                preview: row.get(10)?,
                compaction_count: row.get::<_, i64>(5)? as usize,
                last_compaction_before_tokens: row
                    .get::<_, Option<i64>>(6)?
                    .map(|value| value as usize),
                last_compaction_after_tokens: row
                    .get::<_, Option<i64>>(7)?
                    .map(|value| value as usize),
                last_compaction_recent_file_count: row
                    .get::<_, Option<i64>>(8)?
                    .map(|value| value as usize),
                last_compaction_boundary_version: row
                    .get::<_, Option<i64>>(9)?
                    .map(|value| value as u32),
            })
        })?;
        let mut threads = Vec::new();
        for row in rows {
            threads.push(row?);
        }
        Ok(threads)
    }

    pub fn list_recent_thread_records(
        &self,
        limit: usize,
    ) -> Result<Vec<PersistedRecentThreadRecord>> {
        let conn = self.conn.lock().expect("state db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT s.id, s.cwd, s.branch, s.provider, s.model, s.base_url,
                    s.agent_mode, s.bash_approval, s.history_len, s.transcript_len,
                    s.updated_at, s.compaction_count, s.last_compaction_before_tokens,
                    s.last_compaction_after_tokens, s.last_compaction_recent_file_count,
                    s.last_compaction_boundary_version,
                    COALESCE((
                        SELECT preview FROM turns
                        WHERE session_id = s.id
                        ORDER BY ordinal DESC
                        LIMIT 1
                    ), '') AS preview
             FROM sessions s
             ORDER BY s.updated_at DESC
             LIMIT ?",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(PersistedRecentThreadRecord {
                session_id: row.get(0)?,
                cwd: row.get(1)?,
                branch: row.get(2)?,
                provider: row.get(3)?,
                model: row.get(4)?,
                base_url: row.get(5)?,
                agent_mode: row.get(6)?,
                bash_approval: row.get(7)?,
                history_len: row.get::<_, i64>(8)? as usize,
                transcript_len: row.get::<_, i64>(9)? as usize,
                updated_at: row.get(10)?,
                compaction_count: row.get::<_, i64>(11)? as usize,
                last_compaction_before_tokens: row
                    .get::<_, Option<i64>>(12)?
                    .map(|value| value as usize),
                last_compaction_after_tokens: row
                    .get::<_, Option<i64>>(13)?
                    .map(|value| value as usize),
                last_compaction_recent_file_count: row
                    .get::<_, Option<i64>>(14)?
                    .map(|value| value as usize),
                last_compaction_boundary_version: row
                    .get::<_, Option<i64>>(15)?
                    .map(|value| value as u32),
                preview: row.get(16)?,
            })
        })?;
        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row?);
        }
        Ok(sessions)
    }

    fn write_turn_artifact(
        &self,
        session_id: &str,
        ordinal: usize,
        entries: &[PersistedTurnEntry],
    ) -> Result<String> {
        let relative = self.artifact_relative_path(session_id, ordinal);
        let full_path = self.rollout_root().join(&relative);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(entries)?;
        fs::write(&full_path, content)?;
        Ok(relative.display().to_string())
    }

    fn artifact_relative_path(&self, session_id: &str, ordinal: usize) -> PathBuf {
        PathBuf::from(session_id).join(format!("{ordinal:06}.json"))
    }

    fn runtime_rollout_path(&self, session_id: &str) -> PathBuf {
        self.rollout_root().join(session_id).join("runtime.json")
    }

    fn rollout_events_path(&self, session_id: &str) -> PathBuf {
        self.rollout_root().join(session_id).join("events.json")
    }

    fn write_rollout_events(
        &self,
        session_id: &str,
        items: &[PersistedStructuredRolloutEvent],
    ) -> Result<()> {
        let path = self.rollout_events_path(session_id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(items)?;
        fs::write(path, content)?;
        Ok(())
    }

    fn init_schema(&self) -> Result<()> {
        let conn = self.conn.lock().expect("state db mutex poisoned");
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                cwd TEXT NOT NULL,
                branch TEXT NOT NULL,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                base_url TEXT,
                agent_mode TEXT NOT NULL,
                bash_approval TEXT NOT NULL,
                plan_explanation TEXT,
                history_len INTEGER NOT NULL DEFAULT 0,
                transcript_len INTEGER NOT NULL DEFAULT 0,
                compaction_count INTEGER NOT NULL DEFAULT 0,
                last_compaction_before_tokens INTEGER,
                last_compaction_after_tokens INTEGER,
                last_compaction_recent_file_count INTEGER,
                last_compaction_boundary_version INTEGER,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS turns (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                ordinal INTEGER NOT NULL,
                event_count INTEGER NOT NULL DEFAULT 0,
                artifact_path TEXT NOT NULL,
                preview TEXT NOT NULL DEFAULT '',
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                UNIQUE(session_id, ordinal)
            );

            CREATE TABLE IF NOT EXISTS plan_steps (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                step_index INTEGER NOT NULL,
                status TEXT NOT NULL,
                step TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS interactions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                status TEXT NOT NULL,
                title TEXT NOT NULL,
                summary TEXT NOT NULL,
                payload_json TEXT,
                updated_at INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_turns_session_ordinal
                ON turns(session_id, ordinal);
            CREATE INDEX IF NOT EXISTS idx_plan_steps_session_step
                ON plan_steps(session_id, step_index);
            CREATE INDEX IF NOT EXISTS idx_interactions_session_kind
                ON interactions(session_id, kind);
            ",
        )?;
        ensure_column(&conn, "sessions", "plan_explanation", "TEXT")?;
        ensure_column(
            &conn,
            "sessions",
            "compaction_count",
            "INTEGER NOT NULL DEFAULT 0",
        )?;
        ensure_column(
            &conn,
            "sessions",
            "last_compaction_before_tokens",
            "INTEGER",
        )?;
        ensure_column(&conn, "sessions", "last_compaction_after_tokens", "INTEGER")?;
        ensure_column(
            &conn,
            "sessions",
            "last_compaction_recent_file_count",
            "INTEGER",
        )?;
        ensure_column(
            &conn,
            "sessions",
            "last_compaction_boundary_version",
            "INTEGER",
        )?;
        ensure_column(&conn, "interactions", "payload_json", "TEXT")?;
        Ok(())
    }
}

fn epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn turn_preview(entries: &[PersistedTurnEntry]) -> String {
    entries
        .iter()
        .find_map(|entry| {
            let first_line = entry.message.lines().next()?.trim();
            if first_line.is_empty() {
                None
            } else {
                Some(format!("{}: {}", entry.role, first_line))
            }
        })
        .unwrap_or_else(|| "empty turn".to_string())
}

fn ensure_column(conn: &Connection, table: &str, column: &str, definition: &str) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for row in rows {
        if row? == column {
            return Ok(());
        }
    }
    conn.execute(
        &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
        [],
    )?;
    Ok(())
}
