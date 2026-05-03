use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, bail};

use crate::llm::LlmBackend;
use crate::vectordb::{MemoryMetadata, VectorDB};

const EXPERIENCES_TABLE: &str = "experiences";
const DEFAULT_IMPORTANCE: f32 = 0.5;

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

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MemoryRecord {
    pub id: String,
    pub title: String,
    pub content: String,
    pub labels: Vec<MemoryLabel>,
    pub importance: f32,
    pub source: MemorySource,
    pub scope: MemoryScope,
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
}

impl MemoryStore {
    pub fn new(backend: Arc<dyn LlmBackend>, vdb: Arc<VectorDB>) -> Self {
        Self { backend, vdb }
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
            created_at_unix_seconds: now,
            updated_at_unix_seconds: now,
        };
        let vector = self.backend.embed(&record.content).await?;
        self.vdb
            .upsert_turn(
                EXPERIENCES_TABLE,
                MemoryMetadata {
                    id: Some(record.id.clone()),
                    session_id: memory_scope_key(&record.scope).to_string(),
                    turn_index: stable_memory_index(&record.content),
                    text: record.content.clone(),
                },
                vector,
            )
            .await?;
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
        Ok(hits
            .into_iter()
            .map(|hit| MemoryRecordSearchHit {
                record: MemoryRecord::from(hit.metadata),
                score: hit.score,
                vector_distance: hit.vector_distance,
                fts_score: hit.fts_score,
            })
            .collect())
    }
}

impl From<MemoryMetadata> for MemoryRecord {
    fn from(metadata: MemoryMetadata) -> Self {
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
            created_at_unix_seconds: now,
            updated_at_unix_seconds: now,
        }
    }
}

fn normalized_labels(labels: Vec<MemoryLabel>) -> Vec<MemoryLabel> {
    if labels.is_empty() {
        return vec![MemoryLabel::Experience];
    }
    labels
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

fn stable_memory_index(text: &str) -> u32 {
    text.bytes().fold(2_166_136_261u32, |hash, byte| {
        hash.wrapping_mul(16_777_619) ^ u32::from(byte)
    })
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
            })
            .await
            .expect("insert memory");

        assert_eq!(saved.importance, 1.0);
        assert_eq!(saved.labels, vec![MemoryLabel::Experience]);
        assert_eq!(saved.scope, MemoryScope::Workspace);
    }
}
