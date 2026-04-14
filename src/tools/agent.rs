use crate::tool::{Tool, ToolError, ToolManager};
use crate::agent::{Agent};
use crate::llm::LlmBackend;
use crate::vectordb::VectorDB;
use crate::session::SessionManager;
use crate::workspace::WorkspaceMemory;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;

pub struct AgentTool { pub backend: Arc<dyn LlmBackend>, pub vdb: Arc<VectorDB>, pub session_manager: Arc<SessionManager>, pub workspace: Arc<WorkspaceMemory> }
#[async_trait]
impl Tool for AgentTool {
    fn name(&self) -> &str { "spawn_agent" }
    fn description(&self) -> &str { "Spawn sub-agent" }
    fn input_schema(&self) -> Value { json!({ "type": "object", "properties": { "name": { "type": "string" }, "instruction": { "type": "string" } }, "required": ["name", "instruction"] }) }
    async fn call(&self, i: Value) -> Result<Value, ToolError> {
        let name = i["name"].as_str().unwrap_or("worker");
        let instruction = i["instruction"].as_str().ok_or(ToolError::InvalidInput("instruction".into()))?;
        let mut sub = Agent::new(ToolManager::new(), self.backend.clone(), self.vdb.clone(), self.session_manager.clone(), self.workspace.clone());
        sub.query(instruction.to_string()).await.map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
        Ok(json!({ "name": name, "status": "done" }))
    }
}

pub struct TeamCreateTool { pub backend: Arc<dyn LlmBackend>, pub vdb: Arc<VectorDB>, pub session_manager: Arc<SessionManager>, pub workspace: Arc<WorkspaceMemory> }
#[async_trait]
impl Tool for TeamCreateTool {
    fn name(&self) -> &str { "team_create" }
    fn description(&self) -> &str { "Launch parallel sub-agents" }
    fn input_schema(&self) -> Value { json!({ "type": "object", "properties": { "tasks": { "type": "array" } }, "required": ["tasks"] }) }
    async fn call(&self, i: Value) -> Result<Value, ToolError> {
        let tasks = i["tasks"].as_array().ok_or(ToolError::InvalidInput("tasks".into()))?;
        let mut results = Vec::new();
        for task in tasks {
            let name = task["name"].as_str().unwrap_or("worker");
            results.push(json!({ "name": name, "status": "mocked_done" }));
        }
        Ok(json!({ "team_results": results }))
    }
}
