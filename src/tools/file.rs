use crate::tool::{Tool, ToolError};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;
use walkdir::WalkDir;

pub struct ReadFileTool;
#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }
    fn description(&self) -> &str {
        "Read the content of a file"
    }
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
        let p = i["path"]
            .as_str()
            .ok_or(ToolError::InvalidInput("path".into()))?;
        let content = fs::read_to_string(p)?;
        let lines = content.lines().collect::<Vec<_>>();
        let total_lines = lines.len();
        let start_line = i
            .get("start_line")
            .and_then(Value::as_u64)
            .map(|v| v as usize);
        let end_line = i
            .get("end_line")
            .and_then(Value::as_u64)
            .map(|v| v as usize);

        let sliced_content = match (start_line, end_line) {
            (None, None) => content,
            (start, end) => {
                let start = start.unwrap_or(1);
                let end = end.unwrap_or(total_lines.max(1));
                if start == 0 || end == 0 {
                    return Err(ToolError::InvalidInput(
                        "start_line/end_line must be >= 1".into(),
                    ));
                }
                if start > end {
                    return Err(ToolError::InvalidInput(
                        "start_line must be <= end_line".into(),
                    ));
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
    fn name(&self) -> &str {
        "write_file"
    }
    fn description(&self) -> &str {
        "Write content to a file"
    }
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
        let p = i["path"]
            .as_str()
            .ok_or(ToolError::InvalidInput("path".into()))?;
        let c = i["content"]
            .as_str()
            .ok_or(ToolError::InvalidInput("content".into()))?;
        let existing = existing_file_summary(p)?;
        let operation = if existing.is_some() {
            "overwritten"
        } else {
            "created"
        };
        fs::write(p, c)?;
        Ok(json!({
            "status": "ok",
            "path": p,
            "operation": operation,
            "bytes_written": c.len(),
            "line_count": c.lines().count(),
            "previous_bytes": existing.as_ref().map(|(bytes, _)| *bytes),
            "previous_line_count": existing.as_ref().map(|(_, line_count)| *line_count),
        }))
    }
}

pub struct ReplaceTool;
#[async_trait]
impl Tool for ReplaceTool {
    fn name(&self) -> &str {
        "replace"
    }
    fn description(&self) -> &str {
        "Replace a specific string in a file"
    }
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
        let p = i["path"]
            .as_str()
            .ok_or(ToolError::InvalidInput("path".into()))?;
        let o = i["old_string"]
            .as_str()
            .ok_or(ToolError::InvalidInput("old".into()))?;
        let n = i["new_string"]
            .as_str()
            .ok_or(ToolError::InvalidInput("new".into()))?;
        let c = fs::read_to_string(p)?;
        if c.matches(o).count() != 1 {
            return Err(ToolError::ExecutionFailed("String not unique".into()));
        }
        let updated = c.replace(o, n);
        fs::write(p, &updated)?;
        Ok(json!({
            "status": "ok",
            "path": p,
            "replacements": 1,
            "old_preview": preview_snippet(o),
            "new_preview": preview_snippet(n),
            "old_bytes": o.len(),
            "new_bytes": n.len(),
            "line_delta": updated.lines().count() as i64 - c.lines().count() as i64,
        }))
    }
}

pub struct ListFilesTool;
#[async_trait]
impl Tool for ListFilesTool {
    fn name(&self) -> &str {
        "list_files"
    }
    fn description(&self) -> &str {
        "Recursively list files"
    }
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
        let p = i["path"]
            .as_str()
            .ok_or(ToolError::InvalidInput("path".into()))?;
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

fn preview_snippet(value: &str) -> String {
    let mut preview = value.replace('\n', "\\n");
    const MAX_PREVIEW: usize = 80;
    if preview.chars().count() > MAX_PREVIEW {
        preview = preview.chars().take(MAX_PREVIEW).collect::<String>();
        preview.push_str("...");
    }
    preview
}

fn existing_file_summary(path: &str) -> Result<Option<(u64, usize)>, ToolError> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(ToolError::Io(err)),
    };

    let file = fs::File::open(path)?;
    let reader = BufReader::new(file);
    let mut line_count = 0usize;
    for line in reader.lines() {
        line?;
        line_count += 1;
    }

    Ok(Some((metadata.len(), line_count)))
}

#[cfg(test)]
mod tests {
    use super::{ListFilesTool, ReadFileTool, ReplaceTool, WriteFileTool};
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

    #[tokio::test]
    async fn write_file_reports_created_or_overwritten() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let path = tempdir.path().join("sample.txt");

        let tool = WriteFileTool;
        let created = tool
            .call(json!({
                "path": path.display().to_string(),
                "content": "hello\nworld\n"
            }))
            .await
            .expect("write created");
        assert_eq!(created["operation"], "created");
        assert_eq!(created["line_count"], 2);

        let overwritten = tool
            .call(json!({
                "path": path.display().to_string(),
                "content": "updated\n"
            }))
            .await
            .expect("write overwritten");
        assert_eq!(overwritten["operation"], "overwritten");
        assert_eq!(overwritten["previous_line_count"], 2);
    }

    #[tokio::test]
    async fn replace_reports_preview_and_line_delta() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let path = tempdir.path().join("sample.txt");
        std::fs::write(&path, "hello old value\n").expect("write sample");

        let tool = ReplaceTool;
        let result = tool
            .call(json!({
                "path": path.display().to_string(),
                "old_string": "old value",
                "new_string": "new\nvalue"
            }))
            .await
            .expect("replace content");

        assert_eq!(result["replacements"], 1);
        assert_eq!(result["old_preview"], "old value");
        assert_eq!(result["new_preview"], "new\\nvalue");
        assert_eq!(result["line_delta"], 1);
    }
}
