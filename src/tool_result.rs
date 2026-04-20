use std::fs;
use std::path::PathBuf;

use anyhow::Result;
use serde_json::{json, Value};

use crate::agent::Message;

const INLINE_CHAR_BUDGET: usize = 8_000;
const FILE_LIST_PREVIEW_LIMIT: usize = 200;
const MATCH_PREVIEW_LIMIT: usize = 40;
const LARGE_PREVIEW_HEAD: usize = 4_000;

pub struct ToolResultStore {
    base_dir: PathBuf,
}

impl ToolResultStore {
    pub fn new(base_dir: impl Into<PathBuf>) -> Result<Self> {
        let base_dir = base_dir.into();
        if !base_dir.exists() {
            fs::create_dir_all(&base_dir)?;
        }
        Ok(Self { base_dir })
    }

    pub fn compact_result(
        &self,
        tool_name: &str,
        tool_use_id: &str,
        input: &Value,
        result: &Value,
    ) -> Result<String> {
        let summary = summarize_tool_result(tool_name, input, result);
        let inline = match tool_name {
            "apply_patch" => compact_apply_patch(result),
            "write_file" => compact_write_file(result),
            "replace" => compact_replace(result),
            "list_files" => compact_list_files(input, result),
            "read_file" => compact_read_file(input, result),
            "glob" => compact_glob(result),
            "grep" => compact_grep(result),
            "web_fetch" => compact_web_fetch(input, result),
            _ => compact_generic(&summary, result),
        };
        let full_rendered =
            serde_json::to_string_pretty(result).unwrap_or_else(|_| result.to_string());
        let should_persist = full_rendered.chars().count() > INLINE_CHAR_BUDGET
            || inline.chars().count() > INLINE_CHAR_BUDGET;

        if !should_persist {
            return Ok(inline);
        }

        let stored_path = self.persist_full_result(tool_use_id, tool_name, input, result)?;
        Ok(format!(
            "{}\n\n[tool_result truncated]\nfull_result_path={}",
            truncate_text(&inline, LARGE_PREVIEW_HEAD),
            stored_path.display()
        ))
    }

    fn persist_full_result(
        &self,
        tool_use_id: &str,
        tool_name: &str,
        input: &Value,
        result: &Value,
    ) -> Result<PathBuf> {
        let path = self.base_dir.join(format!("{tool_use_id}.json"));
        let summary = summarize_tool_result(tool_name, input, result);
        let payload = json!({
            "tool_name": tool_name,
            "summary": summary,
            "input": input,
            "result": result,
        });
        fs::write(&path, serde_json::to_string_pretty(&payload)?)?;
        Ok(path)
    }
}

pub fn repair_tool_result_history(history: &[Message]) -> Vec<Message> {
    let mut repaired = Vec::with_capacity(history.len());
    let mut pending_tool_uses: Vec<String> = Vec::new();

    for message in history {
        if message.role == "assistant" {
            if !pending_tool_uses.is_empty() {
                repaired.push(synthetic_tool_result_message(&pending_tool_uses));
                pending_tool_uses.clear();
            }
            pending_tool_uses.extend(extract_tool_use_ids(&message.content));
            repaired.push(message.clone());
            continue;
        }

        if message.role == "user" && has_tool_result_blocks(&message.content) {
            let mut kept_blocks = Vec::new();
            if let Some(items) = message.content.as_array() {
                for item in items {
                    if item.get("type").and_then(Value::as_str) == Some("tool_result") {
                        let Some(tool_use_id) = item.get("tool_use_id").and_then(Value::as_str)
                        else {
                            continue;
                        };
                        if let Some(pos) = pending_tool_uses.iter().position(|id| id == tool_use_id)
                        {
                            pending_tool_uses.remove(pos);
                            kept_blocks.push(item.clone());
                        }
                    } else {
                        kept_blocks.push(item.clone());
                    }
                }
            }
            if !kept_blocks.is_empty() {
                repaired.push(Message {
                    role: message.role.clone(),
                    content: Value::Array(kept_blocks),
                });
            }
            continue;
        }

        if !pending_tool_uses.is_empty() {
            repaired.push(synthetic_tool_result_message(&pending_tool_uses));
            pending_tool_uses.clear();
        }
        repaired.push(message.clone());
    }

    if !pending_tool_uses.is_empty() {
        repaired.push(synthetic_tool_result_message(&pending_tool_uses));
    }

    repaired
}

fn synthetic_tool_result_message(ids: &[String]) -> Message {
    Message {
        role: "user".to_string(),
        content: Value::Array(
            ids.iter()
                .map(|id| {
                    json!({
                        "type": "tool_result",
                        "tool_use_id": id,
                        "content": "Tool execution was interrupted before a result was recorded.",
                        "is_error": true
                    })
                })
                .collect(),
        ),
    }
}

fn extract_tool_use_ids(content: &Value) -> Vec<String> {
    content
        .as_array()
        .into_iter()
        .flatten()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("tool_use"))
        .filter_map(|item| item.get("id").and_then(Value::as_str).map(str::to_string))
        .collect()
}

fn has_tool_result_blocks(content: &Value) -> bool {
    content.as_array().is_some_and(|items| {
        items
            .iter()
            .any(|item| item.get("type").and_then(Value::as_str) == Some("tool_result"))
    })
}

fn compact_list_files(input: &Value, result: &Value) -> String {
    let files = result
        .get("files")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let rendered = files
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join("\n");
    let summary = summarize_tool_result("list_files", input, result);
    format!("{summary}\nPreview:\n{rendered}")
}

fn compact_read_file(input: &Value, result: &Value) -> String {
    let content = result
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let total_chars = content.chars().count();
    let preview = truncate_text(content, INLINE_CHAR_BUDGET.min(LARGE_PREVIEW_HEAD));
    let summary = summarize_tool_result("read_file", input, result);
    if preview.chars().count() < total_chars {
        format!("{summary}\nContent preview:\n{preview}\n... truncated.")
    } else {
        format!("{summary}\nContent:\n{preview}")
    }
}

fn compact_glob(result: &Value) -> String {
    let matches = result
        .get("matches")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let total = matches.len();
    let preview = matches
        .iter()
        .take(FILE_LIST_PREVIEW_LIMIT)
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join("\n");
    let remaining = total.saturating_sub(FILE_LIST_PREVIEW_LIMIT);
    let summary = summarize_tool_result("glob", &Value::Null, result);
    if remaining > 0 {
        format!("{summary}\nPreview:\n{preview}\n... {remaining} more omitted.")
    } else {
        format!("{summary}\nPreview:\n{preview}")
    }
}

fn compact_grep(result: &Value) -> String {
    let matches = result
        .get("results")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let total = matches.len();
    let preview = matches
        .iter()
        .take(MATCH_PREVIEW_LIMIT)
        .map(|entry| {
            let file = entry
                .get("file")
                .and_then(Value::as_str)
                .unwrap_or("<unknown>");
            let line = entry.get("line").and_then(Value::as_u64).unwrap_or(0);
            let content = entry
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default();
            format!("{file}:{line}: {content}")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let remaining = total.saturating_sub(MATCH_PREVIEW_LIMIT);
    let summary = summarize_tool_result("grep", &Value::Null, result);
    if remaining > 0 {
        format!("{summary}\nPreview:\n{preview}\n... {remaining} more omitted.")
    } else {
        format!("{summary}\nPreview:\n{preview}")
    }
}

fn compact_web_fetch(input: &Value, result: &Value) -> String {
    let content = result
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let total_chars = content.chars().count();
    let preview = truncate_text(content, LARGE_PREVIEW_HEAD);
    let summary = summarize_tool_result("web_fetch", input, result);
    if preview.chars().count() < total_chars {
        format!("{summary}\nContent preview:\n{preview}\n... truncated.")
    } else {
        format!("{summary}\nContent:\n{preview}")
    }
}

fn compact_apply_patch(result: &Value) -> String {
    let status = result
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let files_changed = result
        .get("files_changed")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let hunks_applied = result
        .get("hunks_applied")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let summary_items = result
        .get("summary")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let preview = summary_items
        .iter()
        .take(10)
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join("\n");
    let remainder = summary_items.len().saturating_sub(10);
    if remainder > 0 {
        format!(
            "Patch {status}: {files_changed} file(s), {hunks_applied} hunk(s).\nChanges:\n{preview}\n... {remainder} more change(s) omitted."
        )
    } else {
        format!(
            "Patch {status}: {files_changed} file(s), {hunks_applied} hunk(s).\nChanges:\n{preview}"
        )
    }
}

fn compact_generic(summary: &str, result: &Value) -> String {
    let rendered = serde_json::to_string_pretty(result).unwrap_or_else(|_| result.to_string());
    format!(
        "{summary}\nPayload:\n{}",
        truncate_text(&rendered, LARGE_PREVIEW_HEAD)
    )
}

fn compact_write_file(result: &Value) -> String {
    let path = result
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or("<unknown>");
    let operation = result
        .get("operation")
        .and_then(Value::as_str)
        .unwrap_or("updated");
    let line_count = result
        .get("line_count")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let bytes_written = result
        .get("bytes_written")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let mut rendered =
        format!("write_file {operation} {path}\nlines={line_count} bytes={bytes_written}");
    if let Some(previous_bytes) = result.get("previous_bytes").and_then(Value::as_u64) {
        let previous_lines = result
            .get("previous_line_count")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        rendered.push_str(&format!(
            "\nprevious_lines={previous_lines} previous_bytes={previous_bytes}"
        ));
    }
    rendered
}

fn compact_replace(result: &Value) -> String {
    let path = result
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or("<unknown>");
    let replacements = result
        .get("replacements")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let line_delta = result
        .get("line_delta")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    let old_preview = result
        .get("old_preview")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let new_preview = result
        .get("new_preview")
        .and_then(Value::as_str)
        .unwrap_or_default();
    format!(
        "replace {path}\nreplacements={replacements} line_delta={line_delta}\nold={old_preview}\nnew={new_preview}"
    )
}

fn summarize_tool_result(tool_name: &str, input: &Value, result: &Value) -> String {
    match tool_name {
        "list_files" => {
            let root = input.get("path").and_then(Value::as_str).unwrap_or(".");
            let total = result
                .get("files")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            format!("Listed {total} path(s) under {root}.")
        }
        "read_file" => {
            let path = input
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or("<unknown>");
            let total_chars = result
                .get("content")
                .and_then(Value::as_str)
                .map(|content| content.chars().count())
                .unwrap_or_default();
            let total_lines = result
                .get("total_lines")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            let start_line = result
                .get("start_line")
                .and_then(Value::as_u64)
                .unwrap_or(1);
            let end_line = result
                .get("end_line")
                .and_then(Value::as_u64)
                .unwrap_or(total_lines);
            if total_lines > 0 && (start_line != 1 || end_line != total_lines) {
                format!(
                    "Read file {path} lines {start_line}-{end_line} of {total_lines} ({total_chars} chars)."
                )
            } else {
                format!("Read file {path} ({total_lines} lines, {total_chars} chars).")
            }
        }
        "glob" => {
            let total = result
                .get("matches")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            format!("Glob matched {total} path(s).")
        }
        "grep" => {
            let total = result
                .get("results")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            format!("Grep found {total} match(es).")
        }
        "web_fetch" => {
            let url = input
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or("<unknown>");
            let total_chars = result
                .get("content")
                .and_then(Value::as_str)
                .map(|content| content.chars().count())
                .unwrap_or_default();
            format!("Fetched {url} ({total_chars} chars).")
        }
        "apply_patch" => {
            let status = result
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let files_changed = result
                .get("files_changed")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            let hunks_applied = result
                .get("hunks_applied")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            format!("Patch {status}: {files_changed} file(s), {hunks_applied} hunk(s).")
        }
        "write_file" => {
            let path = result
                .get("path")
                .and_then(Value::as_str)
                .or_else(|| input.get("path").and_then(Value::as_str))
                .unwrap_or("<unknown>");
            let operation = result
                .get("operation")
                .and_then(Value::as_str)
                .unwrap_or("updated");
            let line_count = result
                .get("line_count")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            let bytes_written = result
                .get("bytes_written")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            format!("Write file {path}: {operation} ({line_count} lines, {bytes_written} bytes).")
        }
        "replace" => {
            let path = result
                .get("path")
                .and_then(Value::as_str)
                .or_else(|| input.get("path").and_then(Value::as_str))
                .unwrap_or("<unknown>");
            let replacements = result
                .get("replacements")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            format!("Replace in {path}: {replacements} replacement(s).")
        }
        _ => {
            let keys = result
                .as_object()
                .map(|object| object.keys().cloned().collect::<Vec<_>>().join(", "))
                .filter(|keys| !keys.is_empty())
                .unwrap_or_else(|| "scalar result".to_string());
            format!("Tool {tool_name} completed with {keys}.")
        }
    }
}

fn truncate_text(text: &str, max_chars: usize) -> String {
    let mut collected = String::new();
    for ch in text.chars().take(max_chars) {
        collected.push(ch);
    }
    collected
}

pub fn default_tool_result_store_dir() -> Result<PathBuf> {
    let root = std::env::current_dir()?;
    Ok(rara_config::workspace_data_dir_for(&root)?.join("tool-results"))
}

#[cfg(test)]
mod tests {
    use super::{
        compact_read_file, default_tool_result_store_dir, repair_tool_result_history,
        ToolResultStore,
    };
    use crate::agent::Message;
    use serde_json::json;

    #[test]
    fn repairs_missing_tool_results() {
        let history = vec![
            Message {
                role: "assistant".into(),
                content: json!([{ "type": "tool_use", "id": "call-1", "name": "list_files", "input": {} }]),
            },
            Message {
                role: "assistant".into(),
                content: json!([{ "type": "text", "text": "follow-up" }]),
            },
        ];
        let repaired = repair_tool_result_history(&history);
        assert_eq!(repaired.len(), 3);
        assert_eq!(repaired[1].role, "user");
        assert!(repaired[1].content.to_string().contains("call-1"));
    }

    #[test]
    fn compacts_large_read_file_results() {
        let summary = compact_read_file(
            &json!({ "path": "src/main.rs" }),
            &json!({ "content": "a".repeat(10_000) }),
        );
        assert!(summary.contains("Read file src/main.rs"));
        assert!(summary.contains("truncated"));
    }

    #[test]
    fn stores_oversized_results_on_disk() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let store = ToolResultStore::new(tempdir.path()).expect("store");
        let output = store
            .compact_result(
                "web_fetch",
                "tool-1",
                &json!({ "url": "https://example.com" }),
                &json!({ "content": "x".repeat(20_000) }),
            )
            .expect("compact result");
        assert!(output.contains("full_result_path="));
        assert!(output.contains("Fetched https://example.com"));
        assert!(tempdir.path().join("tool-1.json").exists());
        assert!(default_tool_result_store_dir()
            .expect("default tool result dir")
            .ends_with(std::path::Path::new("tool-results")));
    }
}
