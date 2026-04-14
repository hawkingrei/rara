use crate::tool::{Tool, ToolError};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

pub struct ReadFileTool;
#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str { "read_file" }
    fn description(&self) -> &str { "Read the content of a file" }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "start_line": { "type": "integer", "minimum": 1 },
                "end_line": { "type": "integer", "minimum": 1 }
            },
            "required": ["path"]
        })
    }
    async fn call(&self, i: Value) -> Result<Value, ToolError> {
        let p = i["path"].as_str().ok_or(ToolError::InvalidInput("path".into()))?;
        let content = fs::read_to_string(p)?;
        let lines = content.lines().collect::<Vec<_>>();
        let total_lines = lines.len();
        let start_line = i.get("start_line").and_then(Value::as_u64).map(|v| v as usize);
        let end_line = i.get("end_line").and_then(Value::as_u64).map(|v| v as usize);

        let sliced_content = match (start_line, end_line) {
            (None, None) => content,
            (start, end) => {
                let start = start.unwrap_or(1);
                let end = end.unwrap_or(total_lines.max(1));
                if start == 0 || end == 0 {
                    return Err(ToolError::InvalidInput("start_line/end_line must be >= 1".into()));
                }
                if start > end {
                    return Err(ToolError::InvalidInput("start_line must be <= end_line".into()));
                }
                if total_lines == 0 {
                    String::new()
                } else {
                    let bounded_start = start.min(total_lines);
                    let bounded_end = end.min(total_lines);
                    lines[bounded_start - 1..bounded_end].join("\n")
                }
            }
        };

        Ok(json!({
            "content": sliced_content,
            "total_lines": total_lines,
            "start_line": start_line.unwrap_or(1),
            "end_line": end_line.unwrap_or(total_lines),
        }))
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
            "properties": {
                "path": { "type": "string" },
                "include_ignored": { "type": "boolean" }
            },
            "required": ["path"]
        })
    }
    async fn call(&self, i: Value) -> Result<Value, ToolError> {
        let p = i["path"].as_str().ok_or(ToolError::InvalidInput("path".into()))?;
        let include_ignored = i
            .get("include_ignored")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let files: Vec<String> = WalkDir::new(p)
            .into_iter()
            .filter_entry(|entry| include_ignored || !is_ignored_path(entry.path()))
            .filter_map(|e| e.ok())
            .map(|e| e.path().display().to_string())
            .collect();
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

fn is_ignored_path(path: &Path) -> bool {
    path.components().any(|component| {
        let name = component.as_os_str().to_string_lossy();
        matches!(
            name.as_ref(),
            ".git"
                | "target"
                | "node_modules"
                | "dist"
                | "build"
                | ".next"
                | ".cache"
                | "__pycache__"
                | ".venv"
                | "venv"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::{ListFilesTool, ReadFileTool};
    use crate::tool::Tool;
    use serde_json::json;

    #[tokio::test]
    async fn read_file_supports_line_ranges() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let path = tempdir.path().join("sample.txt");
        std::fs::write(&path, "a\nb\nc\nd\n").expect("write sample");

        let tool = ReadFileTool;
        let result = tool
            .call(json!({
                "path": path.display().to_string(),
                "start_line": 2,
                "end_line": 3
            }))
            .await
            .expect("read file");

        assert_eq!(result["content"], "b\nc");
        assert_eq!(result["total_lines"], 4);
        assert_eq!(result["start_line"], 2);
        assert_eq!(result["end_line"], 3);
    }

    #[tokio::test]
    async fn list_files_skips_build_artifacts_by_default() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path();
        std::fs::create_dir_all(root.join("src")).expect("mkdir src");
        std::fs::create_dir_all(root.join("target/debug")).expect("mkdir target");
        std::fs::write(root.join("src/main.rs"), "fn main() {}\n").expect("write source");
        std::fs::write(root.join("target/debug/app"), "bin").expect("write artifact");

        let tool = ListFilesTool;
        let result = tool
            .call(json!({ "path": root.display().to_string() }))
            .await
            .expect("list files");
        let files = result["files"].as_array().expect("files array");
        let rendered = files
            .iter()
            .filter_map(|value| value.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("src/main.rs"));
        assert!(!rendered.contains("target/debug/app"));
    }
}
