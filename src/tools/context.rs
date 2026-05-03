use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::llm::LlmBackend;
use crate::session::SessionManager;
use crate::tool::{Tool, ToolError};
use crate::vectordb::VectorDB;

pub struct RetrieveSessionContextTool {
    pub backend: Arc<dyn LlmBackend>,
    pub vdb: Arc<VectorDB>,
    pub session_manager: Arc<SessionManager>,
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
    async fn call(&self, _: Value) -> Result<Value, ToolError> {
        Ok(json!({ "status": "no_context_found" }))
    }
}
