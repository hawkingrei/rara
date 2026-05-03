use crate::llm::LlmBackend;
use crate::tool::{Tool, ToolError};
use crate::vectordb::{MemoryMetadata, VectorDB};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;

pub struct RememberExperienceTool {
    pub backend: Arc<dyn LlmBackend>,
    pub vdb: Arc<VectorDB>,
    pub db_uri: String,
}
#[async_trait]
impl Tool for RememberExperienceTool {
    fn name(&self) -> &str {
        "remember_experience"
    }
    fn description(&self) -> &str {
        "Save insight"
    }
    fn input_schema(&self) -> Value {
        json!({ "type": "object", "properties": { "experience": { "type": "string" } }, "required": ["experience"] })
    }
    async fn call(&self, i: Value) -> Result<Value, ToolError> {
        let text = i["experience"]
            .as_str()
            .ok_or(ToolError::InvalidInput("experience".into()))?;
        let vector = self
            .backend
            .embed(text)
            .await
            .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
        self.vdb
            .upsert_turn(
                "experiences",
                MemoryMetadata {
                    session_id: "project".to_string(),
                    turn_index: stable_experience_turn_index(text),
                    text: text.to_string(),
                },
                vector,
            )
            .await
            .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
        Ok(json!({ "status": "ok", "saved": text, "store": self.db_uri }))
    }
}

pub struct RetrieveExperienceTool {
    pub backend: Arc<dyn LlmBackend>,
    pub vdb: Arc<VectorDB>,
    pub db_uri: String,
}
#[async_trait]
impl Tool for RetrieveExperienceTool {
    fn name(&self) -> &str {
        "retrieve_experience"
    }
    fn description(&self) -> &str {
        "Retrieve past insights"
    }
    fn input_schema(&self) -> Value {
        json!({ "type": "object", "properties": { "query": { "type": "string" } }, "required": ["query"] })
    }
    async fn call(&self, input: Value) -> Result<Value, ToolError> {
        let query = input["query"]
            .as_str()
            .ok_or(ToolError::InvalidInput("query".into()))?;
        let query_vector = self
            .backend
            .embed(query)
            .await
            .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
        let hits = self
            .vdb
            .hybrid_search_with_metadata("experiences", query, query_vector, 8)
            .await
            .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
        let relevant_experiences = hits
            .iter()
            .map(|hit| hit.metadata.text.clone())
            .collect::<Vec<_>>();
        let diagnostics = hits
            .iter()
            .map(|hit| {
                json!({
                    "session_id": hit.metadata.session_id,
                    "turn_index": hit.metadata.turn_index,
                    "score": hit.score,
                    "vector_distance": hit.vector_distance,
                    "fts_score": hit.fts_score,
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({
            "relevant_experiences": relevant_experiences,
            "diagnostics": diagnostics,
            "store": self.db_uri,
        }))
    }
}

fn stable_experience_turn_index(text: &str) -> u32 {
    text.bytes().fold(2_166_136_261u32, |hash, byte| {
        hash.wrapping_mul(16_777_619) ^ u32::from(byte)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::MockLlm;

    #[tokio::test]
    async fn remember_and_retrieve_experience_use_lancedb_hybrid_index() {
        let temp = tempfile::tempdir().expect("tempdir");
        let vdb = Arc::new(VectorDB::new(temp.path().to_str().expect("utf8 path")));
        let backend = Arc::new(MockLlm);
        let remember = RememberExperienceTool {
            backend: backend.clone(),
            vdb: vdb.clone(),
            db_uri: vdb.uri().to_string(),
        };
        remember
            .call(json!({ "experience": "DeepSeek DSML requires a structured parser." }))
            .await
            .expect("remember experience");

        let retrieve = RetrieveExperienceTool {
            backend,
            vdb,
            db_uri: temp.path().display().to_string(),
        };
        let result = retrieve
            .call(json!({ "query": "DSML parser" }))
            .await
            .expect("retrieve experience");
        let experiences = result["relevant_experiences"]
            .as_array()
            .expect("experience array");
        assert_eq!(experiences.len(), 1);
        assert_eq!(
            experiences[0].as_str(),
            Some("DeepSeek DSML requires a structured parser.")
        );
        assert!(
            result["diagnostics"]
                .as_array()
                .is_some_and(|items| !items.is_empty())
        );
    }
}
