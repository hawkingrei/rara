use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};

use crate::file_lock::AdvisoryFileLock;
use crate::llm::LlmBackend;
use crate::vectordb::{MemoryMetadata, VectorDB};

const EXPERIENCES_TABLE: &str = "experiences";
const DEFAULT_IMPORTANCE: f32 = 0.5;
const MEMORY_RECORD_INDEX_PLACEHOLDER: u32 = 0;
const MEMORY_RECORDS_FILE_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryLabel {
    Insight,
    Decision,
    Fact,
    Procedure,
    Experience,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemorySource {
    AgentTurn,
    UserCreated,
    ThreadDistill,
    FileImport,
    ProtocolWrite,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryScope {
    User,
    Workspace,
    Project,
    Thread,
    Session,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MemorySourceSpan {
    pub start_turn_index: u32,
    pub end_turn_index: u32,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MemoryRecord {
    pub id: String,
    pub title: String,
    pub content: String,
    pub labels: Vec<MemoryLabel>,
    pub importance: f32,
    pub source: MemorySource,
    pub scope: MemoryScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_span: Option<MemorySourceSpan>,
    pub created_at_unix_seconds: u64,
    pub updated_at_unix_seconds: u64,
}

#[derive(Debug, Clone)]
pub struct NewMemoryRecord {
    pub title: Option<String>,
    pub content: String,
    pub labels: Vec<MemoryLabel>,
    pub importance: f32,
    pub source: MemorySource,
    pub scope: MemoryScope,
    pub session_id: Option<String>,
    pub thread_id: Option<String>,
    pub source_span: Option<MemorySourceSpan>,
}

impl NewMemoryRecord {
    pub fn experience(content: impl Into<String>) -> Self {
        Self {
            title: None,
            content: content.into(),
            labels: vec![MemoryLabel::Experience],
            importance: DEFAULT_IMPORTANCE,
            source: MemorySource::AgentTurn,
            scope: MemoryScope::Project,
            session_id: None,
            thread_id: None,
            source_span: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MemoryRecordSearchHit {
    pub record: MemoryRecord,
    pub score: f32,
    pub vector_distance: Option<f32>,
    pub fts_score: Option<f32>,
}

pub struct MemoryStore {
    backend: Arc<dyn LlmBackend>,
    vdb: Arc<VectorDB>,
    records: MemoryRecordFileStore,
}

impl MemoryStore {
    pub fn new(backend: Arc<dyn LlmBackend>, vdb: Arc<VectorDB>) -> Self {
        let records = MemoryRecordFileStore::for_vdb_uri(vdb.uri());
        Self {
            backend,
            vdb,
            records,
        }
    }

    pub fn new_with_record_path(
        backend: Arc<dyn LlmBackend>,
        vdb: Arc<VectorDB>,
        record_path: PathBuf,
    ) -> Self {
        Self {
            backend,
            vdb,
            records: MemoryRecordFileStore::new(record_path),
        }
    }

    pub async fn insert(&self, input: NewMemoryRecord) -> Result<MemoryRecord> {
        let content = input.content.trim();
        if content.is_empty() {
            bail!("memory content must not be empty");
        }
        let importance = clamp_importance(input.importance);
        let now = unix_timestamp_seconds();
        let record = MemoryRecord {
            id: format!("memory-{}", uuid::Uuid::new_v4()),
            title: input
                .title
                .filter(|title| !title.trim().is_empty())
                .unwrap_or_else(|| title_from_content(content)),
            content: content.to_string(),
            labels: normalized_labels(input.labels),
            importance,
            source: input.source,
            scope: input.scope,
            session_id: normalized_optional_id(input.session_id),
            thread_id: normalized_optional_id(input.thread_id),
            source_span: input.source_span,
            created_at_unix_seconds: now,
            updated_at_unix_seconds: now,
        };
        let vector = self.backend.embed(&record.content).await?;
        self.vdb
            .upsert_turn(
                EXPERIENCES_TABLE,
                MemoryMetadata {
                    id: Some(record.id.clone()),
                    session_id: record.index_scope_key(),
                    turn_index: MEMORY_RECORD_INDEX_PLACEHOLDER,
                    text: record.content.clone(),
                },
                vector,
            )
            .await?;
        self.records.upsert(&record).await?;
        Ok(record)
    }

    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryRecordSearchHit>> {
        if query.trim().is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let query_vector = self.backend.embed(query).await?;
        let hits = self
            .vdb
            .hybrid_search_with_metadata(EXPERIENCES_TABLE, query, query_vector, limit)
            .await?;
        let records = self.records.load_map().await?;
        Ok(hits
            .into_iter()
            .map(|hit| MemoryRecordSearchHit {
                record: memory_record_for_hit(&records, &hit.metadata),
                score: hit.score,
                vector_distance: hit.vector_distance,
                fts_score: hit.fts_score,
            })
            .collect())
    }

    pub async fn get(&self, id: &str) -> Result<Option<MemoryRecord>> {
        self.records.get(id).await
    }
}

impl MemoryRecord {
    fn index_scope_key(&self) -> String {
        self.session_id
            .clone()
            .or_else(|| self.thread_id.clone())
            .unwrap_or_else(|| memory_scope_key(&self.scope).to_string())
    }
}

impl From<MemoryMetadata> for MemoryRecord {
    fn from(metadata: MemoryMetadata) -> Self {
        let session_id = metadata.session_id.clone();
        let turn_index = metadata.turn_index;
        let now = unix_timestamp_seconds();
        Self {
            id: metadata.id.unwrap_or_else(|| {
                format!("legacy-{}-{}", metadata.session_id, metadata.turn_index)
            }),
            title: title_from_content(&metadata.text),
            content: metadata.text,
            labels: vec![MemoryLabel::Experience],
            importance: DEFAULT_IMPORTANCE,
            source: MemorySource::AgentTurn,
            scope: memory_scope_from_key(&metadata.session_id),
            session_id: Some(session_id),
            thread_id: None,
            source_span: Some(MemorySourceSpan {
                start_turn_index: turn_index,
                end_turn_index: turn_index,
            }),
            created_at_unix_seconds: now,
            updated_at_unix_seconds: now,
        }
    }
}

#[derive(Debug, Clone)]
struct MemoryRecordFileStore {
    path: PathBuf,
    lock_path: PathBuf,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct PersistedMemoryRecordFile {
    version: u32,
    records: Vec<MemoryRecord>,
}

impl Default for PersistedMemoryRecordFile {
    fn default() -> Self {
        Self {
            version: MEMORY_RECORDS_FILE_VERSION,
            records: Vec::new(),
        }
    }
}

impl MemoryRecordFileStore {
    fn new(path: PathBuf) -> Self {
        Self {
            lock_path: path.with_extension("json.lock"),
            path,
        }
    }

    fn for_vdb_uri(uri: &str) -> Self {
        Self::new(default_record_path_for_vdb_uri(uri))
    }

    async fn upsert(&self, record: &MemoryRecord) -> Result<()> {
        let path = self.path.clone();
        let lock_path = self.lock_path.clone();
        let record = record.clone();
        tokio::task::spawn_blocking(move || upsert_record_sync(path, lock_path, record))
            .await
            .context("join memory record persistence task")?
    }

    async fn load_map(&self) -> Result<HashMap<String, MemoryRecord>> {
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || load_records_sync(&path))
            .await
            .context("join memory record load task")?
            .map(|records| {
                records
                    .into_iter()
                    .map(|record| (record.id.clone(), record))
                    .collect()
            })
    }

    async fn get(&self, id: &str) -> Result<Option<MemoryRecord>> {
        let id = id.to_string();
        Ok(self.load_map().await?.remove(&id))
    }
}

fn default_record_path_for_vdb_uri(uri: &str) -> PathBuf {
    let db_path = PathBuf::from(uri);
    if db_path.file_name().and_then(|value| value.to_str()) == Some("lancedb") {
        return db_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("memories")
            .join("records.json");
    }
    db_path.join("memory_records.json")
}

fn upsert_record_sync(path: PathBuf, lock_path: PathBuf, record: MemoryRecord) -> Result<()> {
    let _lock = AdvisoryFileLock::acquire(lock_path)?;
    let mut file = PersistedMemoryRecordFile {
        records: load_records_sync(&path)?,
        ..Default::default()
    };
    if let Some(existing) = file.records.iter_mut().find(|item| item.id == record.id) {
        *existing = record;
    } else {
        file.records.push(record);
    }
    write_record_file_sync(&path, &file)
}

fn load_records_sync(path: &Path) -> Result<Vec<MemoryRecord>> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => {
            return Err(err).with_context(|| format!("read memory records {}", path.display()));
        }
    };
    if content.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str::<PersistedMemoryRecordFile>(&content)
        .map(|file| file.records)
        .or_else(|_| serde_json::from_str::<Vec<MemoryRecord>>(&content))
        .with_context(|| format!("parse memory records {}", path.display()))
}

fn write_record_file_sync(path: &Path, file: &PersistedMemoryRecordFile) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create memory records dir {}", parent.display()))?;
    }
    let tmp_path = path.with_extension(format!("json.tmp-{}", uuid::Uuid::new_v4()));
    fs::write(&tmp_path, serde_json::to_string_pretty(file)?)
        .with_context(|| format!("write memory records temp file {}", tmp_path.display()))?;
    if let Err(err) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(err).with_context(|| format!("replace memory records {}", path.display()));
    }
    Ok(())
}

fn memory_record_for_hit(
    records: &HashMap<String, MemoryRecord>,
    metadata: &MemoryMetadata,
) -> MemoryRecord {
    metadata
        .id
        .as_ref()
        .and_then(|id| records.get(id))
        .cloned()
        .unwrap_or_else(|| MemoryRecord::from(metadata.clone()))
}

fn normalized_labels(labels: Vec<MemoryLabel>) -> Vec<MemoryLabel> {
    if labels.is_empty() {
        return vec![MemoryLabel::Experience];
    }
    labels
}

fn normalized_optional_id(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn clamp_importance(importance: f32) -> f32 {
    if importance.is_nan() {
        return DEFAULT_IMPORTANCE;
    }
    importance.clamp(0.1, 1.0)
}

fn memory_scope_key(scope: &MemoryScope) -> &'static str {
    match scope {
        MemoryScope::User => "user",
        MemoryScope::Workspace => "workspace",
        MemoryScope::Project => "project",
        MemoryScope::Thread => "thread",
        MemoryScope::Session => "session",
    }
}

fn memory_scope_from_key(value: &str) -> MemoryScope {
    match value {
        "user" => MemoryScope::User,
        "workspace" => MemoryScope::Workspace,
        "thread" => MemoryScope::Thread,
        "session" => MemoryScope::Session,
        _ => MemoryScope::Project,
    }
}

fn title_from_content(content: &str) -> String {
    let first_line = content
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("Memory");
    let title = first_line
        .split_terminator(['.', '!', '?'])
        .next()
        .unwrap_or(first_line)
        .trim();
    truncate_chars(title, 80)
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

fn unix_timestamp_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::MockLlm;

    #[tokio::test]
    async fn memory_store_inserts_and_searches_memory_records() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = MemoryStore::new(
            Arc::new(MockLlm),
            Arc::new(VectorDB::new(temp.path().to_str().expect("utf8 path"))),
        );

        let saved = store
            .insert(NewMemoryRecord::experience(
                "DeepSeek DSML requires a structured parser.",
            ))
            .await
            .expect("insert memory");
        assert!(saved.id.starts_with("memory-"));
        assert_eq!(saved.title, "DeepSeek DSML requires a structured parser");
        assert_eq!(saved.labels, vec![MemoryLabel::Experience]);

        let hits = store
            .search("DSML parser", 8)
            .await
            .expect("search memories");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].record.id, saved.id);
        assert_eq!(
            hits[0].record.content,
            "DeepSeek DSML requires a structured parser."
        );
        assert_eq!(
            store.get(&saved.id).await.expect("get saved memory"),
            Some(saved)
        );
    }

    #[tokio::test]
    async fn memory_store_clamps_importance_and_defaults_labels() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = MemoryStore::new(
            Arc::new(MockLlm),
            Arc::new(VectorDB::new(temp.path().to_str().expect("utf8 path"))),
        );

        let saved = store
            .insert(NewMemoryRecord {
                title: Some("".to_string()),
                content: "A durable fact".to_string(),
                labels: Vec::new(),
                importance: 4.0,
                source: MemorySource::UserCreated,
                scope: MemoryScope::Workspace,
                session_id: None,
                thread_id: None,
                source_span: None,
            })
            .await
            .expect("insert memory");

        assert_eq!(saved.importance, 1.0);
        assert_eq!(saved.labels, vec![MemoryLabel::Experience]);
        assert_eq!(saved.scope, MemoryScope::Workspace);
    }

    #[tokio::test]
    async fn memory_store_persists_thread_provenance_across_instances() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db_path = temp.path().join("lancedb");
        let record_path = temp.path().join("memories").join("records.json");
        let backend = Arc::new(MockLlm);
        let vdb = Arc::new(VectorDB::new(db_path.to_str().expect("utf8 path")));
        let store =
            MemoryStore::new_with_record_path(backend.clone(), vdb.clone(), record_path.clone());

        let saved = store
            .insert(NewMemoryRecord {
                title: Some("Thread decision".to_string()),
                content: "Keep memory retrieval behind MemoryStore.".to_string(),
                labels: vec![MemoryLabel::Decision],
                importance: 0.9,
                source: MemorySource::ThreadDistill,
                scope: MemoryScope::Thread,
                session_id: Some("session-123".to_string()),
                thread_id: Some("thread-123".to_string()),
                source_span: Some(MemorySourceSpan {
                    start_turn_index: 2,
                    end_turn_index: 4,
                }),
            })
            .await
            .expect("insert memory");

        let reloaded = MemoryStore::new_with_record_path(backend, vdb, record_path);
        let hits = reloaded
            .search("memory retrieval", 5)
            .await
            .expect("search memories");
        assert_eq!(hits[0].record.id, saved.id);
        assert_eq!(hits[0].record.title, "Thread decision");
        assert_eq!(hits[0].record.labels, vec![MemoryLabel::Decision]);
        assert_eq!(hits[0].record.importance, 0.9);
        assert_eq!(hits[0].record.source, MemorySource::ThreadDistill);
        assert_eq!(hits[0].record.scope, MemoryScope::Thread);
        assert_eq!(hits[0].record.session_id.as_deref(), Some("session-123"));
        assert_eq!(hits[0].record.thread_id.as_deref(), Some("thread-123"));
        assert_eq!(
            hits[0].record.source_span,
            Some(MemorySourceSpan {
                start_turn_index: 2,
                end_turn_index: 4,
            })
        );
    }
}
