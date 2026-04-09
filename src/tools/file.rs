use crate::tool::{Tool, ToolError};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::fs;
use walkdir::WalkDir;

pub struct ReadFileTool;
#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str { "read_file" }
    fn description(&self) -> &str { "Read the content of a file" }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"]
        })
    }
    async fn call(&self, i: Value) -> Result<Value, ToolError> {
        let p = i["path"].as_str().ok_or(ToolError::InvalidInput("path".into()))?;
        Ok(json!({ "content": fs::read_to_string(p)? }))
    }
}

pub struct WriteFileTool;
#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str { "write_file" }
    fn description(&self) -> &str { "Write content to a file" }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "content": { "type": "string" }
            },
            "required": ["path", "content"]
        })
    }
    async fn call(&self, i: Value) -> Result<Value, ToolError> {
        let p = i["path"].as_str().ok_or(ToolError::InvalidInput("path".into()))?;
        let c = i["content"].as_str().ok_or(ToolError::InvalidInput("content".into()))?;
        fs::write(p, c)?; Ok(json!({ "status": "ok" }))
    }
}

pub struct ReplaceTool;
#[async_trait]
impl Tool for ReplaceTool {
    fn name(&self) -> &str { "replace" }
    fn description(&self) -> &str { "Replace a specific string in a file" }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "old_string": { "type": "string" },
                "new_string": { "type": "string" }
            },
            "required": ["path", "old_string", "new_string"]
        })
    }
    async fn call(&self, i: Value) -> Result<Value, ToolError> {
        let p = i["path"].as_str().ok_or(ToolError::InvalidInput("path".into()))?;
        let o = i["old_string"].as_str().ok_or(ToolError::InvalidInput("old".into()))?;
        let n = i["new_string"].as_str().ok_or(ToolError::InvalidInput("new".into()))?;
        let c = fs::read_to_string(p)?;
        if c.matches(o).count() != 1 { return Err(ToolError::ExecutionFailed("String not unique".into())); }
        fs::write(p, c.replace(o, n))?; Ok(json!({ "status": "ok" }))
    }
}

pub struct ListFilesTool;
#[async_trait]
impl Tool for ListFilesTool {
    fn name(&self) -> &str { "list_files" }
    fn description(&self) -> &str { "Recursively list files" }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"]
        })
    }
    async fn call(&self, i: Value) -> Result<Value, ToolError> {
        let p = i["path"].as_str().ok_or(ToolError::InvalidInput("path".into()))?;
        let files: Vec<String> = WalkDir::new(p).into_iter().filter_map(|e| e.ok()).map(|e| e.path().display().to_string()).collect();
        Ok(json!({ "files": files }))
    }
}

pub struct SearchFilesTool;
#[async_trait]
impl Tool for SearchFilesTool {
    fn name(&self) -> &str { "search_files" }
    fn description(&self) -> &str { "Search for pattern in files" }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "pattern": { "type": "string" }
            },
            "required": ["path", "pattern"]
        })
    }
    async fn call(&self, _i: Value) -> Result<Value, ToolError> {
        Ok(json!({ "status": "not_fully_implemented" }))
    }
}
