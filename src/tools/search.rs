use crate::tool::{Tool, ToolError};
use async_trait::async_trait;
use serde_json::{Value, json};
use glob::glob;
use regex::Regex;
use std::fs;

pub struct GlobTool;
#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str { "glob" }
    fn description(&self) -> &str { "Find files matching glob pattern" }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "pattern": { "type": "string" } },
            "required": ["pattern"]
        })
    }
    async fn call(&self, i: Value) -> Result<Value, ToolError> {
        let p = i["pattern"].as_str().ok_or(ToolError::InvalidInput("pattern".into()))?;
        let mut matches = Vec::new();
        for entry in glob(p).map_err(|e| ToolError::InvalidInput(e.to_string()))? {
            if let Ok(path) = entry { matches.push(path.display().to_string()); }
        }
        Ok(json!({ "matches": matches }))
    }
}

pub struct GrepTool;
#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str { "grep" }
    fn description(&self) -> &str { "Regex search in files" }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string" },
                "path": { "type": "string", "default": "." }
            },
            "required": ["pattern"]
        })
    }
    async fn call(&self, i: Value) -> Result<Value, ToolError> {
        let p = i["pattern"].as_str().ok_or(ToolError::InvalidInput("pattern".into()))?;
        let search_path = i["path"].as_str().unwrap_or(".");
        let re = Regex::new(p).map_err(|e| ToolError::InvalidInput(e.to_string()))?;
        let mut results = Vec::new();
        for entry in walkdir::WalkDir::new(search_path).into_iter().filter_map(|e| e.ok()) {
            if entry.file_type().is_file() {
                if let Ok(c) = fs::read_to_string(entry.path()) {
                    for (line_idx, line) in c.lines().enumerate() {
                        if re.is_match(line) {
                            results.push(json!({ "file": entry.path().display().to_string(), "line": line_idx + 1, "content": line.trim() }));
                        }
                    }
                }
            }
            if results.len() > 100 { break; }
        }
        Ok(json!({ "results": results }))
    }
}
