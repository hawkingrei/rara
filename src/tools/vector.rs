use crate::llm::LlmBackend;
use crate::tool::{Tool, ToolError};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

pub struct RememberExperienceTool {
    pub backend: Arc<dyn LlmBackend>,
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
        Ok(json!({ "status": "ok", "saved": text }))
    }
}

pub struct RetrieveExperienceTool {
    pub backend: Arc<dyn LlmBackend>,
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
    async fn call(&self, _: Value) -> Result<Value, ToolError> {
        Ok(json!({ "relevant_experiences": [] }))
    }
}
