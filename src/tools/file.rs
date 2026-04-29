use crate::tool::{Tool, ToolError};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, BufReader as AsyncBufReader};
use walkdir::WalkDir;

const DEFAULT_READ_LINE_LIMIT: usize = 2_000;
const MAX_READ_LINE_CHARS: usize = 4_000;
const MAX_READ_LINE_BYTES: usize = MAX_READ_LINE_CHARS * 4;

pub struct ReadFileTool;
#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }
    fn description(&self) -> &str {
        "Read a file with Codex-style 1-based offset/limit windows. Defaults to the first 2000 lines and reports next_offset when more content remains."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to read." },
                "offset": { "type": "integer", "minimum": 1, "description": "Optional 1-based first line to include. Use next_offset from a truncated result to continue." },
                "limit": { "type": "integer", "minimum": 1, "description": "Optional maximum number of lines to return. Defaults to 2000." },
                "start_line": { "type": "integer", "minimum": 1, "description": "Legacy alias for offset." },
                "end_line": { "type": "integer", "minimum": 1, "description": "Legacy 1-based inclusive last line. Prefer offset/limit for new calls." }
            },
            "required": ["path"]
        })
    }
    async fn call(&self, i: Value) -> Result<Value, ToolError> {
        let p = i["path"]
            .as_str()
            .ok_or(ToolError::InvalidInput("path".into()))?;
        let window = read_file_window_from_input(&i)?;
        let output = read_file_window(p, window).await?;

        Ok(json!({
            "content": output.content,
            "total_lines": output.total_lines,
            "total_lines_exact": output.total_lines_exact,
            "observed_lines": output.observed_lines,
            "start_line": output.start_line,
            "end_line": output.end_line,
            "offset": output.start_line,
            "limit": output.limit,
            "num_lines": output.num_lines,
            "truncated": output.truncated,
            "next_offset": output.next_offset,
            "line_truncated": output.line_truncated,
            "line_format": "raw",
            "bytes_read": output.bytes_read,
        }))
    }
}

#[derive(Clone, Copy)]
struct ReadFileWindow {
    offset: usize,
    limit: usize,
}

struct ReadFileOutput {
    content: String,
    total_lines: Option<usize>,
    total_lines_exact: bool,
    observed_lines: usize,
    start_line: usize,
    end_line: usize,
    limit: usize,
    num_lines: usize,
    truncated: bool,
    next_offset: Option<usize>,
    line_truncated: bool,
    bytes_read: usize,
}

struct BoundedLineRead {
    bytes_read: usize,
    eof: bool,
    truncated: bool,
}

fn read_file_window_from_input(input: &Value) -> Result<ReadFileWindow, ToolError> {
    let offset = optional_positive_usize(input, "offset")?;
    let limit = optional_positive_usize(input, "limit")?;
    let start_line = optional_positive_usize(input, "start_line")?;
    let end_line = optional_positive_usize(input, "end_line")?;

    if (offset.is_some() || limit.is_some()) && (start_line.is_some() || end_line.is_some()) {
        return Err(ToolError::InvalidInput(
            "Use either offset/limit or start_line/end_line, not both".into(),
        ));
    }

    if let Some(offset) = offset {
        return Ok(ReadFileWindow {
            offset,
            limit: limit.unwrap_or(DEFAULT_READ_LINE_LIMIT),
        });
    }

    if let Some(limit) = limit {
        return Ok(ReadFileWindow { offset: 1, limit });
    }

    if start_line.is_some() || end_line.is_some() {
        let start = start_line.unwrap_or(1);
        let limit = match end_line {
            Some(end) if start > end => {
                return Err(ToolError::InvalidInput(
                    "start_line must be <= end_line".into(),
                ));
            }
            Some(end) => end - start + 1,
            None => DEFAULT_READ_LINE_LIMIT,
        };
        return Ok(ReadFileWindow {
            offset: start,
            limit,
        });
    }

    Ok(ReadFileWindow {
        offset: 1,
        limit: DEFAULT_READ_LINE_LIMIT,
    })
}

fn optional_positive_usize(input: &Value, key: &str) -> Result<Option<usize>, ToolError> {
    let Some(value) = input.get(key) else {
        return Ok(None);
    };
    let Some(value) = value.as_u64() else {
        return Err(ToolError::InvalidInput(format!("{key} must be an integer")));
    };
    if value == 0 {
        return Err(ToolError::InvalidInput(format!("{key} must be >= 1")));
    }
    Ok(Some(value as usize))
}

async fn read_file_window(path: &str, window: ReadFileWindow) -> Result<ReadFileOutput, ToolError> {
    let file = tokio::fs::File::open(path).await?;
    let mut reader = AsyncBufReader::new(file);
    let mut line = String::new();
    let mut observed_lines = 0usize;
    let mut total_lines_exact = false;
    let mut selected = Vec::new();
    let mut line_truncated = false;
    let mut bytes_read = 0usize;
    let last_requested_line = window
        .offset
        .checked_add(window.limit - 1)
        .ok_or_else(|| ToolError::InvalidInput("offset + limit overflows".into()))?;

    loop {
        line.clear();
        let read = read_bounded_line(&mut reader, &mut line).await?;
        if read.eof {
            total_lines_exact = true;
            break;
        }
        let bytes = read.bytes_read;
        bytes_read += bytes;
        observed_lines += 1;
        line_truncated |= read.truncated;

        if observed_lines > last_requested_line {
            break;
        }

        if observed_lines < window.offset {
            continue;
        }

        let text = line.trim_end_matches(['\r', '\n']);
        let (text, truncated) = truncate_read_line(text);
        line_truncated |= truncated;
        selected.push(text);
    }

    if window.offset > observed_lines.max(1) {
        return Err(ToolError::ExecutionFailed(format!(
            "offset {} exceeds file length {}",
            window.offset, observed_lines
        )));
    }

    let num_lines = selected.len();
    let end_line = if num_lines == 0 {
        0
    } else {
        window.offset + num_lines - 1
    };
    let has_more_lines = observed_lines > end_line;
    let next_offset = if has_more_lines {
        Some(end_line + 1)
    } else {
        None
    };

    Ok(ReadFileOutput {
        content: selected.join("\n"),
        total_lines: total_lines_exact.then_some(observed_lines),
        total_lines_exact,
        observed_lines,
        start_line: window.offset,
        end_line,
        limit: window.limit,
        num_lines,
        truncated: has_more_lines || line_truncated,
        next_offset,
        line_truncated,
        bytes_read,
    })
}

async fn read_bounded_line<R>(
    reader: &mut R,
    line: &mut String,
) -> Result<BoundedLineRead, ToolError>
where
    R: AsyncBufRead + Unpin,
{
    let mut captured = Vec::new();
    let mut bytes_read = 0usize;
    let mut truncated = false;
    line.clear();

    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            *line = String::from_utf8_lossy(&captured).into_owned();
            return Ok(BoundedLineRead {
                bytes_read,
                eof: bytes_read == 0,
                truncated,
            });
        }

        let newline_pos = available.iter().position(|byte| *byte == b'\n');
        let chunk_len = newline_pos.map_or(available.len(), |pos| pos + 1);
        let remaining = MAX_READ_LINE_BYTES.saturating_sub(captured.len());
        if remaining > 0 {
            let take = remaining.min(chunk_len);
            captured.extend_from_slice(&available[..take]);
            truncated |= take < chunk_len;
        } else {
            truncated = true;
        }
        bytes_read += chunk_len;
        reader.consume(chunk_len);

        if newline_pos.is_some() {
            *line = String::from_utf8_lossy(&captured).into_owned();
            return Ok(BoundedLineRead {
                bytes_read,
                eof: false,
                truncated,
            });
        }
    }
}

fn truncate_read_line(line: &str) -> (String, bool) {
    if line.chars().count() <= MAX_READ_LINE_CHARS {
        return (line.to_string(), false);
    }

    let mut truncated = line.chars().take(MAX_READ_LINE_CHARS).collect::<String>();
    truncated.push_str("... [line truncated]");
    (truncated, true)
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
    use super::{
        ListFilesTool, MAX_READ_LINE_BYTES, MAX_READ_LINE_CHARS, ReadFileTool, ReplaceLinesTool,
        ReplaceTool, WriteFileTool,
    };
    use crate::tool::Tool;
    use serde_json::{Value, json};

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
        assert_eq!(result["total_lines"], Value::Null);
        assert_eq!(result["total_lines_exact"], false);
        assert_eq!(result["observed_lines"], 4);
        assert_eq!(result["start_line"], 2);
        assert_eq!(result["end_line"], 3);
        assert_eq!(result["offset"], 2);
        assert_eq!(result["limit"], 2);
        assert_eq!(result["num_lines"], 2);
        assert_eq!(result["truncated"], true);
        assert_eq!(result["next_offset"], 4);
    }

    #[tokio::test]
    async fn read_file_supports_offset_limit_windows() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let path = tempdir.path().join("sample.txt");
        std::fs::write(&path, "alpha\nbeta\ngamma\n").expect("write sample");

        let tool = ReadFileTool;
        let result = tool
            .call(json!({
                "path": path.display().to_string(),
                "offset": 2,
                "limit": 1
            }))
            .await
            .expect("read file");

        assert_eq!(result["content"], "beta");
        assert_eq!(result["total_lines"], Value::Null);
        assert_eq!(result["total_lines_exact"], false);
        assert_eq!(result["observed_lines"], 3);
        assert_eq!(result["start_line"], 2);
        assert_eq!(result["end_line"], 2);
        assert_eq!(result["num_lines"], 1);
        assert_eq!(result["truncated"], true);
        assert_eq!(result["next_offset"], 3);
    }

    #[tokio::test]
    async fn read_file_defaults_to_limited_first_window() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let path = tempdir.path().join("large.txt");
        let content = (1..=2002)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&path, content).expect("write sample");

        let tool = ReadFileTool;
        let result = tool
            .call(json!({ "path": path.display().to_string() }))
            .await
            .expect("read file");

        assert!(result["content"].as_str().unwrap().contains("line 1"));
        assert!(result["content"].as_str().unwrap().contains("line 2000"));
        assert!(!result["content"].as_str().unwrap().contains("line 2001"));
        assert_eq!(result["total_lines"], Value::Null);
        assert_eq!(result["total_lines_exact"], false);
        assert_eq!(result["observed_lines"], 2001);
        assert_eq!(result["start_line"], 1);
        assert_eq!(result["end_line"], 2000);
        assert_eq!(result["limit"], 2000);
        assert_eq!(result["num_lines"], 2000);
        assert_eq!(result["truncated"], true);
        assert_eq!(result["next_offset"], 2001);
    }

    #[tokio::test]
    async fn read_file_errors_when_offset_exceeds_file_length() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let path = tempdir.path().join("sample.txt");
        std::fs::write(&path, "one\ntwo\n").expect("write sample");

        let tool = ReadFileTool;
        let error = tool
            .call(json!({
                "path": path.display().to_string(),
                "offset": 3
            }))
            .await
            .expect_err("offset should fail");

        assert!(error.to_string().contains("offset 3 exceeds file length 2"));
    }

    #[tokio::test]
    async fn read_file_marks_truncated_long_lines() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let path = tempdir.path().join("sample.txt");
        std::fs::write(&path, "x".repeat(MAX_READ_LINE_CHARS + 1)).expect("write sample");

        let tool = ReadFileTool;
        let result = tool
            .call(json!({ "path": path.display().to_string() }))
            .await
            .expect("read file");

        assert_eq!(result["total_lines"], 1);
        assert_eq!(result["total_lines_exact"], true);
        assert_eq!(result["line_truncated"], true);
        assert_eq!(result["truncated"], true);
        assert!(
            result["content"]
                .as_str()
                .unwrap()
                .ends_with("... [line truncated]")
        );
    }

    #[tokio::test]
    async fn read_file_bounds_memory_for_very_long_lines() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let path = tempdir.path().join("sample.txt");
        let content = format!("{}\nsecond\n", "x".repeat(MAX_READ_LINE_BYTES * 2));
        std::fs::write(&path, content).expect("write sample");

        let tool = ReadFileTool;
        let result = tool
            .call(json!({
                "path": path.display().to_string(),
                "offset": 1,
                "limit": 1
            }))
            .await
            .expect("read file");

        let content = result["content"].as_str().expect("content");
        assert!(content.ends_with("... [line truncated]"));
        assert!(content.chars().count() < MAX_READ_LINE_BYTES);
        assert_eq!(result["line_truncated"], true);
        assert_eq!(result["truncated"], true);
        assert_eq!(result["next_offset"], 2);
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
        assert!(
            replace_lines_tool
                .description()
                .contains("verifying the current line numbers")
        );
    }
}
