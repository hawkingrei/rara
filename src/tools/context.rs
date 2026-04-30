use crate::experience_store::ExperienceStore;
use crate::tool::{Tool, ToolError};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;

pub struct RetrieveSessionContextTool {
    pub store: Arc<ExperienceStore>,
}
#[async_trait]
impl Tool for RetrieveSessionContextTool {
    fn name(&self) -> &str {
        "retrieve_session_context"
    }
    fn description(&self) -> &str {
        "Recall past context"
    }
    fn input_schema(&self) -> Value {
        json!({ "type": "object", "properties": { "query": { "type": "string" } }, "required": ["query"] })
    }
    async fn call(&self, i: Value) -> Result<Value, ToolError> {
        let query = i["query"]
            .as_str()
            .ok_or(ToolError::InvalidInput("query".into()))?;
        let experiences = self.store.retrieve(query, 5);
        if experiences.is_empty() {
            Ok(json!({ "status": "no_context_found" }))
        } else {
            Ok(json!({ "recalled_context": experiences }))
        }
    }
}
