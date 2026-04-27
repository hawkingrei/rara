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
        "Read a file, optionally with 1-based inclusive line ranges for large files"
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to read." },
                "start_line": { "type": "integer", "minimum": 1, "description": "Optional 1-based first line to include." },
                "end_line": { "type": "integer", "minimum": 1, "description": "Optional 1-based last line to include." }
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
        "Create a new file or intentionally rewrite a whole file. For existing files, read the file first and prefer apply_patch for partial edits."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to create or fully rewrite." },
                "content": { "type": "string", "description": "Complete new file contents. Do not use for small edits to existing files." }
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
        "Replace one exact, unique string in a file. Read the file first and prefer apply_patch for structured multi-line edits."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to edit." },
                "old_string": { "type": "string", "description": "Exact text to replace. It must appear exactly once in the file." },
                "new_string": { "type": "string", "description": "Replacement text." }
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

pub struct ReplaceLinesTool;
#[async_trait]
impl Tool for ReplaceLinesTool {
    fn name(&self) -> &str {
        "replace_lines"
    }
    fn description(&self) -> &str {
        "Replace an inclusive line range in a file. Use only after reading the target range and verifying the current line numbers."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to edit." },
                "start_line": { "type": "integer", "minimum": 1, "description": "1-based first line to replace." },
                "end_line": { "type": "integer", "minimum": 1, "description": "1-based last line to replace, inclusive." },
                "new_string": {
                    "type": "string",
                    "description": "Replacement text for the inclusive line range. Use an empty string to delete the range."
                }
            },
            "required": ["path", "start_line", "end_line", "new_string"]
        })
    }
    async fn call(&self, i: Value) -> Result<Value, ToolError> {
        let path = i["path"]
            .as_str()
            .ok_or(ToolError::InvalidInput("path".into()))?;
        let start_line = i["start_line"]
            .as_u64()
            .ok_or(ToolError::InvalidInput("start_line".into()))? as usize;
        let end_line = i["end_line"]
            .as_u64()
            .ok_or(ToolError::InvalidInput("end_line".into()))? as usize;
        let new_string = i["new_string"]
            .as_str()
            .ok_or(ToolError::InvalidInput("new_string".into()))?;
        if start_line == 0 || end_line == 0 {
            return Err(ToolError::InvalidInput(
                "start_line/end_line must be >= 1".into(),
            ));
        }
        if start_line > end_line {
            return Err(ToolError::InvalidInput(
                "start_line must be <= end_line".into(),
            ));
        }

        let original = fs::read_to_string(path)?;
        let had_trailing_newline = original.ends_with('\n');
        let mut lines = original
            .lines()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let total_lines = lines.len();
        if total_lines == 0 {
            return Err(ToolError::ExecutionFailed(format!(
                "Cannot replace lines in empty file {path}"
            )));
        }
        if end_line > total_lines {
            return Err(ToolError::ExecutionFailed(format!(
                "Line range {start_line}-{end_line} exceeds file length {total_lines}"
            )));
        }

        let replacement_lines = if new_string.is_empty() {
            Vec::new()
        } else {
            new_string
                .lines()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        };
        let removed_line_count = end_line - start_line + 1;
        lines.splice(start_line - 1..end_line, replacement_lines.iter().cloned());

        let mut updated = lines.join("\n");
        if had_trailing_newline && !updated.is_empty() {
            updated.push('\n');
        }
        fs::write(path, updated)?;

        Ok(json!({
            "status": "ok",
            "path": path,
            "start_line": start_line,
            "end_line": end_line,
            "removed_lines": removed_line_count,
            "inserted_lines": replacement_lines.len(),
            "line_delta": replacement_lines.len() as i64 - removed_line_count as i64,
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
    use super::{ListFilesTool, ReadFileTool, ReplaceLinesTool, ReplaceTool, WriteFileTool};
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

    #[tokio::test]
    async fn replace_lines_replaces_inclusive_range_without_old_string() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let path = tempdir.path().join("sample.txt");
        std::fs::write(&path, "one\ntwo\nthree\nfour\n").expect("write sample");

        let tool = ReplaceLinesTool;
        let result = tool
            .call(json!({
                "path": path.display().to_string(),
                "start_line": 2,
                "end_line": 3,
                "new_string": "middle"
            }))
            .await
            .expect("replace lines");

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "one\nmiddle\nfour\n"
        );
        assert_eq!(result["removed_lines"], 2);
        assert_eq!(result["inserted_lines"], 1);
        assert_eq!(result["line_delta"], -1);
    }

    #[test]
    fn file_tool_descriptions_encode_safe_edit_contract() {
        let write_tool = WriteFileTool;
        let replace_tool = ReplaceTool;
        let replace_lines_tool = ReplaceLinesTool;

        assert!(write_tool.description().contains("Create a new file"));
        assert!(write_tool.description().contains("read the file first"));
        assert!(replace_tool.description().contains("exact, unique string"));
        assert!(replace_tool.description().contains("Read the file first"));
        assert!(replace_lines_tool
            .description()
            .contains("verifying the current line numbers"));
    }
}
