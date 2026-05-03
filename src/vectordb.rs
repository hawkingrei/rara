use std::sync::Arc;

use anyhow::{Context, Result};
use arrow_array::cast::AsArray;
use arrow_array::types::{Float32Type, UInt32Type};
use arrow_array::{FixedSizeListArray, Float32Array, RecordBatch, StringArray, UInt32Array};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use futures::TryStreamExt;
use lance_index::scalar::FullTextSearchQuery;
use lancedb::index::Index;
use lancedb::index::scalar::FtsIndexBuilder;
use lancedb::query::{ExecutableQuery, QueryBase, QueryExecutionOptions};
use lancedb::{Table, connect};

const MEMORY_ID_COLUMN: &str = "id";
const SESSION_ID_COLUMN: &str = "session_id";
const TURN_INDEX_COLUMN: &str = "turn_index";
const TEXT_COLUMN: &str = "text";
const VECTOR_COLUMN: &str = "vector";
const VECTOR_DISTANCE_COLUMN: &str = "_distance";
const FTS_SCORE_COLUMN: &str = "_score";

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone, PartialEq, Eq)]
pub struct MemoryMetadata {
    pub session_id: String,
    pub turn_index: u32,
    pub text: String,
}

impl MemoryMetadata {
    fn stable_id(&self) -> String {
        format!("{}:{}", self.session_id, self.turn_index)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MemorySearchHit {
    pub metadata: MemoryMetadata,
    pub score: f32,
    pub vector_distance: Option<f32>,
    pub fts_score: Option<f32>,
}

pub struct VectorDB {
    uri: String,
}

impl VectorDB {
    pub fn new(uri: &str) -> Self {
        Self {
            uri: uri.to_string(),
        }
    }

    pub fn uri(&self) -> &str {
        &self.uri
    }

    pub async fn search_with_metadata(
        &self,
        table_name: &str,
        query_vector: Vec<f32>,
        limit: usize,
    ) -> Result<Vec<(MemoryMetadata, f32)>> {
        if query_vector.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let Some(table) = self.open_table_if_exists(table_name).await? else {
            return Ok(Vec::new());
        };
        let batches = table
            .query()
            .nearest_to(query_vector)?
            .limit(limit)
            .execute()
            .await?
            .try_collect::<Vec<_>>()
            .await?;
        Ok(memory_hits_from_batches(&batches)?
            .into_iter()
            .map(|hit| (hit.metadata, hit.score))
            .collect())
    }

    pub async fn full_text_search_with_metadata(
        &self,
        table_name: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<MemorySearchHit>> {
        if query.trim().is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let Some(table) = self.open_table_if_exists(table_name).await? else {
            return Ok(Vec::new());
        };
        self.ensure_fts_index(&table).await?;
        let batches = table
            .query()
            .full_text_search(FullTextSearchQuery::new(query.to_string()))
            .limit(limit)
            .execute()
            .await?
            .try_collect::<Vec<_>>()
            .await?;
        memory_hits_from_batches(&batches)
    }

    pub async fn hybrid_search_with_metadata(
        &self,
        table_name: &str,
        query: &str,
        query_vector: Vec<f32>,
        limit: usize,
    ) -> Result<Vec<MemorySearchHit>> {
        if query.trim().is_empty() || query_vector.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let Some(table) = self.open_table_if_exists(table_name).await? else {
            return Ok(Vec::new());
        };
        self.ensure_fts_index(&table).await?;
        let batches = table
            .query()
            .full_text_search(FullTextSearchQuery::new(query.to_string()))
            .nearest_to(query_vector)?
            .limit(limit)
            .execute_hybrid(QueryExecutionOptions::default())
            .await?
            .try_collect::<Vec<_>>()
            .await?;
        memory_hits_from_batches(&batches)
    }

    pub async fn upsert_turn(
        &self,
        table_name: &str,
        metadata: MemoryMetadata,
        vector: Vec<f32>,
    ) -> Result<()> {
        if vector.is_empty() {
            return Ok(());
        }
        let table = self.open_or_create_table(table_name, vector.len()).await?;
        let id = metadata.stable_id();
        let delete_filter = format!("{MEMORY_ID_COLUMN} = '{}'", escape_lance_sql_string(&id));
        table.delete(&delete_filter).await?;
        table
            .add(memory_record_batch(&metadata, vector)?)
            .execute()
            .await?;
        self.ensure_fts_index(&table).await?;
        Ok(())
    }

    async fn open_or_create_table(&self, table_name: &str, vector_dim: usize) -> Result<Table> {
        let db = connect(&self.uri)
            .execute()
            .await
            .with_context(|| format!("connect LanceDB at {}", self.uri))?;
        let names = db.table_names().execute().await?;
        if names.iter().any(|name| name == table_name) {
            return db
                .open_table(table_name)
                .execute()
                .await
                .with_context(|| format!("open LanceDB table {table_name}"));
        }
        db.create_empty_table(table_name, memory_schema(vector_dim))
            .execute()
            .await
            .with_context(|| format!("create LanceDB table {table_name}"))
    }

    async fn open_table_if_exists(&self, table_name: &str) -> Result<Option<Table>> {
        let db = connect(&self.uri)
            .execute()
            .await
            .with_context(|| format!("connect LanceDB at {}", self.uri))?;
        let names = db.table_names().execute().await?;
        if !names.iter().any(|name| name == table_name) {
            return Ok(None);
        }
        let table = db
            .open_table(table_name)
            .execute()
            .await
            .with_context(|| format!("open LanceDB table {table_name}"))?;
        Ok(Some(table))
    }

    async fn ensure_fts_index(&self, table: &Table) -> Result<()> {
        let indices = table.list_indices().await?;
        let has_text_fts = indices.iter().any(|index| {
            index.columns.iter().any(|column| column == TEXT_COLUMN)
                && index.index_type.to_string() == "FTS"
        });
        if has_text_fts {
            return Ok(());
        }
        table
            .create_index(&[TEXT_COLUMN], Index::FTS(FtsIndexBuilder::default()))
            .execute()
            .await
            .context("create LanceDB FTS index on memory text")?;
        Ok(())
    }
}

fn memory_schema(vector_dim: usize) -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new(MEMORY_ID_COLUMN, DataType::Utf8, false),
        Field::new(SESSION_ID_COLUMN, DataType::Utf8, false),
        Field::new(TURN_INDEX_COLUMN, DataType::UInt32, false),
        Field::new(TEXT_COLUMN, DataType::Utf8, false),
        Field::new(
            VECTOR_COLUMN,
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                vector_dim as i32,
            ),
            false,
        ),
    ]))
}

fn memory_record_batch(metadata: &MemoryMetadata, vector: Vec<f32>) -> Result<RecordBatch> {
    let schema = memory_schema(vector.len());
    let id = metadata.stable_id();
    let vectors = FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
        [Some(vector.into_iter().map(Some).collect::<Vec<_>>())],
        schema
            .field_with_name(VECTOR_COLUMN)?
            .data_type()
            .as_fixed_size_list()
            .map(|(_, dim)| dim)
            .unwrap_or_default(),
    );
    Ok(RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec![id])),
            Arc::new(StringArray::from(vec![metadata.session_id.clone()])),
            Arc::new(UInt32Array::from(vec![metadata.turn_index])),
            Arc::new(StringArray::from(vec![metadata.text.clone()])),
            Arc::new(vectors),
        ],
    )?)
}

fn memory_hits_from_batches(batches: &[RecordBatch]) -> Result<Vec<MemorySearchHit>> {
    let mut hits = Vec::new();
    for batch in batches {
        let session_ids = string_column(batch, SESSION_ID_COLUMN)?;
        let turn_indices = batch
            .column_by_name(TURN_INDEX_COLUMN)
            .context("missing turn_index column in LanceDB memory result")?
            .as_primitive::<UInt32Type>();
        let texts = string_column(batch, TEXT_COLUMN)?;
        let vector_distances = optional_f32_column(batch, VECTOR_DISTANCE_COLUMN);
        let fts_scores = optional_f32_column(batch, FTS_SCORE_COLUMN);
        for row in 0..batch.num_rows() {
            let vector_distance = vector_distances.as_ref().and_then(|values| values[row]);
            let fts_score = fts_scores.as_ref().and_then(|values| values[row]);
            hits.push(MemorySearchHit {
                metadata: MemoryMetadata {
                    session_id: session_ids.value(row).to_string(),
                    turn_index: turn_indices.value(row),
                    text: texts.value(row).to_string(),
                },
                score: combined_score(vector_distance, fts_score),
                vector_distance,
                fts_score,
            });
        }
    }
    Ok(hits)
}

fn string_column<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a StringArray> {
    batch
        .column_by_name(name)
        .with_context(|| format!("missing {name} column in LanceDB memory result"))?
        .as_any()
        .downcast_ref::<StringArray>()
        .with_context(|| format!("LanceDB memory column {name} is not Utf8"))
}

fn optional_f32_column(batch: &RecordBatch, name: &str) -> Option<Vec<Option<f32>>> {
    let column = batch.column_by_name(name)?;
    if let Some(values) = column.as_any().downcast_ref::<Float32Array>() {
        return Some(values.iter().collect());
    }
    None
}

fn combined_score(vector_distance: Option<f32>, fts_score: Option<f32>) -> f32 {
    match (vector_distance, fts_score) {
        (Some(distance), Some(score)) => score + (1.0 / (1.0 + distance.max(0.0))),
        (Some(distance), None) => 1.0 / (1.0 + distance.max(0.0)),
        (None, Some(score)) => score,
        (None, None) => 0.0,
    }
}

fn escape_lance_sql_string(value: &str) -> String {
    value.replace('\'', "''")
}

trait FixedSizeListDataTypeExt {
    fn as_fixed_size_list(&self) -> Option<(&Field, i32)>;
}

impl FixedSizeListDataTypeExt for DataType {
    fn as_fixed_size_list(&self) -> Option<(&Field, i32)> {
        match self {
            DataType::FixedSizeList(field, dim) => Some((field.as_ref(), *dim)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn lancedb_memory_index_supports_vector_fts_and_hybrid_search() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db = VectorDB::new(temp.path().to_str().expect("utf8 path"));
        db.upsert_turn(
            "conversations",
            MemoryMetadata {
                session_id: "session-a".to_string(),
                turn_index: 1,
                text: "Fix DeepSeek DSML parsing with a structured parser".to_string(),
            },
            vec![1.0, 0.0, 0.0],
        )
        .await
        .expect("insert first memory");
        db.upsert_turn(
            "conversations",
            MemoryMetadata {
                session_id: "session-b".to_string(),
                turn_index: 2,
                text: "Improve viewport rendering for queued approvals".to_string(),
            },
            vec![0.0, 1.0, 0.0],
        )
        .await
        .expect("insert second memory");

        let fts_hits = db
            .full_text_search_with_metadata("conversations", "DeepSeek DSML", 5)
            .await
            .expect("fts search");
        assert_eq!(fts_hits[0].metadata.session_id, "session-a");
        assert!(fts_hits[0].fts_score.is_some());

        let vector_hits = db
            .search_with_metadata("conversations", vec![0.0, 1.0, 0.0], 1)
            .await
            .expect("vector search");
        assert_eq!(vector_hits[0].0.session_id, "session-b");

        let hybrid_hits = db
            .hybrid_search_with_metadata("conversations", "DeepSeek parser", vec![1.0, 0.0, 0.0], 5)
            .await
            .expect("hybrid search");
        assert!(
            hybrid_hits
                .iter()
                .any(|hit| hit.metadata.session_id == "session-a")
        );
    }

    #[tokio::test]
    async fn lancedb_memory_search_does_not_create_table_with_default_dimension() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db = VectorDB::new(temp.path().to_str().expect("utf8 path"));

        let empty = db
            .full_text_search_with_metadata("experiences", "missing", 5)
            .await
            .expect("missing table search");
        assert!(empty.is_empty());

        db.upsert_turn(
            "experiences",
            MemoryMetadata {
                session_id: "session-a".to_string(),
                turn_index: 1,
                text: "Remember exact config key names".to_string(),
            },
            vec![1.0, 0.0, 0.0, 0.0],
        )
        .await
        .expect("insert after missing search should use real vector dimension");
        let hits = db
            .search_with_metadata("experiences", vec![1.0, 0.0, 0.0, 0.0], 1)
            .await
            .expect("vector search");
        assert_eq!(hits[0].0.text, "Remember exact config key names");
    }
}
