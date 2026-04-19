use crate::tool::{Tool, ToolError};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

pub struct ApplyPatchTool;

#[derive(Debug)]
enum PatchOp {
    AddFile {
        path: String,
        lines: Vec<String>,
    },
    DeleteFile {
        path: String,
    },
    UpdateFile {
        path: String,
        move_to: Option<String>,
        chunks: Vec<Chunk>,
    },
}

#[derive(Debug)]
struct Chunk {
    lines: Vec<DiffLine>,
}

#[derive(Debug)]
struct DiffLine {
    kind: char,
    text: String,
}

#[derive(Default)]
struct PatchStats {
    files_changed: usize,
    hunks_applied: usize,
    created_files: Vec<String>,
    deleted_files: Vec<String>,
    moved_files: Vec<Value>,
    updated_files: Vec<String>,
    added_lines: usize,
    removed_lines: usize,
}

#[async_trait]
impl Tool for ApplyPatchTool {
    fn name(&self) -> &str {
        "apply_patch"
    }

    fn description(&self) -> &str {
        "Apply structured file edits using Begin Patch syntax. Prefer this for editing existing files."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "patch": {
                    "type": "string",
                    "description": "Patch text using *** Begin Patch / *** End Patch syntax."
                },
                "dry_run": {
                    "type": "boolean",
                    "description": "Validate and preview without writing files."
                }
            },
            "required": ["patch"]
        })
    }

    async fn call(&self, input: Value) -> Result<Value, ToolError> {
        let patch = input
            .get("patch")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidInput("patch".into()))?;
        let dry_run = input
            .get("dry_run")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let ops = parse_patch(patch)?;
        let mut stats = PatchStats::default();
        let mut previews = Vec::new();

        for op in ops {
            match op {
                PatchOp::AddFile { path, lines } => {
                    if Path::new(&path).exists() {
                        return Err(ToolError::ExecutionFailed(format!(
                            "Cannot add existing file {path}"
                        )));
                    }
                    stats.files_changed += 1;
                    stats.created_files.push(path.clone());
                    stats.hunks_applied += 1;
                    stats.added_lines += lines.len();
                    previews.push(format!("Add file {path}"));
                    if !dry_run {
                        write_text_file(&path, &join_lines(&lines))?;
                    }
                }
                PatchOp::DeleteFile { path } => {
                    if !Path::new(&path).exists() {
                        return Err(ToolError::ExecutionFailed(format!(
                            "Cannot delete missing file {path}"
                        )));
                    }
                    let removed_lines = read_lines(&path)?.len();
                    stats.files_changed += 1;
                    stats.deleted_files.push(path.clone());
                    stats.hunks_applied += 1;
                    stats.removed_lines += removed_lines;
                    previews.push(format!("Delete file {path}"));
                    if !dry_run {
                        fs::remove_file(&path)?;
                    }
                }
                PatchOp::UpdateFile {
                    path,
                    move_to,
                    chunks,
                } => {
                    let original = fs::read_to_string(&path)?;
                    let updated = apply_update_chunks(&path, &original, &chunks, &mut stats)?;
                    stats.files_changed += 1;
                    stats.updated_files.push(path.clone());
                    previews.push(format!(
                        "Update file {}{}",
                        path,
                        move_to
                            .as_ref()
                            .map(|target| format!(" -> {target}"))
                            .unwrap_or_default()
                    ));
                    if let Some(target) = &move_to {
                        stats
                            .moved_files
                            .push(json!({ "from": path, "to": target }));
                    }
                    if !dry_run {
                        let write_path = move_to.as_deref().unwrap_or(&path);
                        if let Some(target) = &move_to {
                            if target != &path {
                                if let Some(parent) = Path::new(target).parent() {
                                    fs::create_dir_all(parent)?;
                                }
                                fs::remove_file(&path)?;
                            }
                        }
                        write_text_file(write_path, &updated)?;
                    }
                }
            }
        }

        Ok(json!({
            "status": if dry_run { "validated" } else { "applied" },
            "files_changed": stats.files_changed,
            "hunks_applied": stats.hunks_applied,
            "created_files": stats.created_files,
            "deleted_files": stats.deleted_files,
            "moved_files": stats.moved_files,
            "updated_files": stats.updated_files,
            "line_delta": {
                "added": stats.added_lines,
                "removed": stats.removed_lines,
            },
            "summary": previews,
            "diff_preview": previews.join("\n"),
        }))
    }
}

fn parse_patch(patch: &str) -> Result<Vec<PatchOp>, ToolError> {
    let lines: Vec<&str> = patch.lines().collect();
    if lines.first().copied() != Some("*** Begin Patch") {
        return Err(ToolError::InvalidInput(
            "Patch must start with *** Begin Patch".into(),
        ));
    }
    if lines.last().copied() != Some("*** End Patch") {
        return Err(ToolError::InvalidInput(
            "Patch must end with *** End Patch".into(),
        ));
    }

    let mut ops = Vec::new();
    let mut index = 1usize;
    while index + 1 < lines.len() {
        let line = lines[index];
        if let Some(path) = line.strip_prefix("*** Add File: ") {
            index += 1;
            let mut add_lines = Vec::new();
            while index < lines.len() && !lines[index].starts_with("*** ") {
                let content = lines[index];
                let Some(text) = content.strip_prefix('+') else {
                    return Err(ToolError::InvalidInput(format!(
                        "Add file entries must start with '+': {content}"
                    )));
                };
                add_lines.push(text.to_string());
                index += 1;
            }
            ops.push(PatchOp::AddFile {
                path: path.to_string(),
                lines: add_lines,
            });
            continue;
        }

        if let Some(path) = line.strip_prefix("*** Delete File: ") {
            ops.push(PatchOp::DeleteFile {
                path: path.to_string(),
            });
            index += 1;
            continue;
        }

        if let Some(path) = line.strip_prefix("*** Update File: ") {
            index += 1;
            let mut move_to = None;
            if index < lines.len() {
                if let Some(target) = lines[index].strip_prefix("*** Move to: ") {
                    move_to = Some(target.to_string());
                    index += 1;
                }
            }

            let mut chunks = Vec::new();
            let mut current_chunk: Option<Chunk> = None;
            while index < lines.len() && !lines[index].starts_with("*** ") {
                let current = lines[index];
                if current.starts_with("@@") {
                    if let Some(chunk) = current_chunk.take() {
                        chunks.push(chunk);
                    }
                    current_chunk = Some(Chunk { lines: Vec::new() });
                } else if current == "*** End of File" {
                } else {
                    let kind = current.chars().next().ok_or_else(|| {
                        ToolError::InvalidInput("Unexpected empty patch line".into())
                    })?;
                    if !matches!(kind, ' ' | '+' | '-') {
                        return Err(ToolError::InvalidInput(format!(
                            "Unexpected patch line: {current}"
                        )));
                    }
                    let chunk = current_chunk.get_or_insert_with(|| Chunk { lines: Vec::new() });
                    chunk.lines.push(DiffLine {
                        kind,
                        text: current[1..].to_string(),
                    });
                }
                index += 1;
            }
            if let Some(chunk) = current_chunk.take() {
                chunks.push(chunk);
            }
            ops.push(PatchOp::UpdateFile {
                path: path.to_string(),
                move_to,
                chunks,
            });
            continue;
        }

        return Err(ToolError::InvalidInput(format!(
            "Unexpected patch directive: {line}"
        )));
    }

    Ok(ops)
}

fn apply_update_chunks(
    path: &str,
    original: &str,
    chunks: &[Chunk],
    stats: &mut PatchStats,
) -> Result<String, ToolError> {
    let original_lines = split_lines(original);
    let mut output = Vec::new();
    let mut cursor = 0usize;

    for chunk in chunks {
        let old_lines: Vec<String> = chunk
            .lines
            .iter()
            .filter(|line| line.kind != '+')
            .map(|line| line.text.clone())
            .collect();
        let new_lines: Vec<String> = chunk
            .lines
            .iter()
            .filter(|line| line.kind != '-')
            .map(|line| line.text.clone())
            .collect();

        let Some(relative_start) = find_subsequence(&original_lines[cursor..], &old_lines) else {
            return Err(ToolError::ExecutionFailed(format!(
                "Patch hunk did not match file {path}"
            )));
        };
        let start = cursor + relative_start;
        output.extend_from_slice(&original_lines[cursor..start]);
        output.extend(new_lines.clone());
        cursor = start + old_lines.len();
        stats.hunks_applied += 1;
        stats.added_lines += chunk.lines.iter().filter(|line| line.kind == '+').count();
        stats.removed_lines += chunk.lines.iter().filter(|line| line.kind == '-').count();
    }

    output.extend_from_slice(&original_lines[cursor..]);
    Ok(join_lines(&output))
}

fn split_lines(text: &str) -> Vec<String> {
    text.lines().map(str::to_string).collect()
}

fn join_lines(lines: &[String]) -> String {
    if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    }
}

fn find_subsequence(haystack: &[String], needle: &[String]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn write_text_file(path: &str, content: &str) -> Result<(), ToolError> {
    if let Some(parent) = Path::new(path).parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(())
}

fn read_lines(path: &str) -> Result<Vec<String>, ToolError> {
    Ok(split_lines(&fs::read_to_string(path)?))
}

#[cfg(test)]
mod tests {
    use super::ApplyPatchTool;
    use crate::tool::Tool;
    use serde_json::json;

    #[tokio::test]
    async fn applies_update_patch() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("sample.txt");
        std::fs::write(&file, "hello\nworld\n").expect("write");

        let tool = ApplyPatchTool;
        let result = tool
            .call(json!({
                "patch": format!(
                    "*** Begin Patch\n*** Update File: {}\n@@\n-hello\n+hi\n world\n*** End Patch",
                    file.display()
                )
            }))
            .await
            .expect("apply patch");

        assert_eq!(std::fs::read_to_string(&file).expect("read"), "hi\nworld\n");
        assert_eq!(result["status"], "applied");
        assert_eq!(result["files_changed"], 1);
    }

    #[tokio::test]
    async fn supports_dry_run() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("sample.txt");
        std::fs::write(&file, "hello\nworld\n").expect("write");

        let tool = ApplyPatchTool;
        let result = tool
            .call(json!({
                "patch": format!(
                    "*** Begin Patch\n*** Update File: {}\n@@\n-hello\n+hi\n world\n*** End Patch",
                    file.display()
                ),
                "dry_run": true
            }))
            .await
            .expect("validate patch");

        assert_eq!(
            std::fs::read_to_string(&file).expect("read"),
            "hello\nworld\n"
        );
        assert_eq!(result["status"], "validated");
    }

    #[tokio::test]
    async fn creates_new_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("created.txt");

        let tool = ApplyPatchTool;
        let result = tool
            .call(json!({
                "patch": format!(
                    "*** Begin Patch\n*** Add File: {}\n+hello\n+world\n*** End Patch",
                    file.display()
                )
            }))
            .await
            .expect("create file");

        assert_eq!(
            std::fs::read_to_string(&file).expect("read"),
            "hello\nworld\n"
        );
        assert_eq!(result["status"], "applied");
        assert_eq!(result["created_files"][0], file.display().to_string());
    }
}
