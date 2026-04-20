use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedTurnEntry {
    pub role: String,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct PersistedPlanStep {
    pub step_index: usize,
    pub status: String,
    pub step: String,
}

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
pub struct PersistedSessionSummary {
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

    pub fn latest_session_id(&self) -> Result<Option<String>> {
        let conn = self.conn.lock().expect("state db mutex poisoned");
        let session_id = conn.query_row(
            "SELECT id FROM sessions ORDER BY updated_at DESC LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        );
        match session_id {
            Ok(id) => Ok(Some(id)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    pub fn list_recent_sessions(&self, limit: usize) -> Result<Vec<PersistedSessionSummary>> {
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
            Ok(PersistedSessionSummary {
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

#[cfg(test)]
mod tests {
    use super::{
        PersistedCompactState, PersistedInteraction, PersistedPlanStep, PersistedTurnEntry, StateDb,
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
        assert_eq!(
            payload.get("allow_net").and_then(|v| v.as_bool()),
            Some(true)
        );
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
        assert_eq!(
            payload.get("allow_net").and_then(|v| v.as_bool()),
            Some(false)
        );
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

        let sessions = db.list_recent_sessions(5)?;
        let summary = sessions
            .iter()
            .find(|item| item.session_id == "session-compact")
            .expect("session summary");
        assert_eq!(summary.compaction_count, 3);
        assert_eq!(summary.last_compaction_before_tokens, Some(12_000));
        assert_eq!(summary.last_compaction_after_tokens, Some(4_200));
        assert_eq!(summary.last_compaction_recent_file_count, Some(2));
        assert_eq!(summary.last_compaction_boundary_version, Some(1));
        Ok(())
    }
}
