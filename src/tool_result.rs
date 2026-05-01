use std::fs;
use std::path::PathBuf;

use anyhow::Result;
use serde_json::{Value, json};

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
            "bash" => compact_bash(result),
            "apply_patch" => compact_apply_patch(result),
            "write_file" => compact_write_file(result),
            "replace" => compact_replace(input, result),
            "replace_lines" => compact_replace_lines(input, result),
            "spawn_agent" | "explore_agent" | "plan_agent" => {
                compact_subagent_result(tool_name, result)
            }
            "list_files" => compact_list_files(input, result),
            "read_file" => compact_read_file(input, result),
            "glob" => compact_glob(result),
            "grep" => compact_grep(result),
            "web_fetch" => compact_web_fetch(input, result),
            "web_search" => compact_web_search(input, result),
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

fn compact_web_search(input: &Value, result: &Value) -> String {
    let content = result
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let total_chars = content.chars().count();
    let preview = truncate_text(content, LARGE_PREVIEW_HEAD);
    let summary = summarize_tool_result("web_search", input, result);
    if preview.chars().count() < total_chars {
        format!("{summary}\nResults preview:\n{preview}\n... truncated.")
    } else {
        format!("{summary}\nResults:\n{preview}")
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

fn compact_bash(result: &Value) -> String {
    if let Some(task_id) = result.get("background_task_id").and_then(Value::as_str) {
        let status = result
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let output_path = result
            .get("output_path")
            .and_then(Value::as_str)
            .unwrap_or("<unknown>");
        let exit_code = result
            .get("exit_code")
            .and_then(Value::as_i64)
            .map(|code| code.to_string())
            .unwrap_or_else(|| "pending".to_string());
        return format!(
            "bash started in background.\nTask id: {task_id}\nStatus: {status}\nExit code: {exit_code}\nOutput path: {output_path}\nUse background_task_status with this task id to inspect output."
        );
    }

    let exit_code = result
        .get("exit_code")
        .and_then(Value::as_i64)
        .map(|code| code.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let duration_ms = result.get("duration_ms").and_then(Value::as_u64);
    let output = result
        .get("aggregated_output")
        .and_then(Value::as_str)
        .filter(|output| !output.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            let stdout = result
                .get("stdout")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let stderr = result
                .get("stderr")
                .and_then(Value::as_str)
                .unwrap_or_default();
            match (stdout.is_empty(), stderr.is_empty()) {
                (true, true) => String::new(),
                (false, true) => stdout.to_string(),
                (true, false) => format!("[stderr] {stderr}"),
                (false, false) => {
                    let separator = if stdout.ends_with('\n') { "" } else { "\n" };
                    format!("{stdout}{separator}[stderr] {stderr}")
                }
            }
        });
    let mut rendered = format!("bash finished.\nExit code: {exit_code}");
    if let Some(duration_ms) = duration_ms {
        rendered.push_str(&format!("\nDuration: {duration_ms} ms"));
    }
    rendered.push_str("\nOutput:\n");
    rendered.push_str(&output);
    rendered
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

fn compact_replace(input: &Value, result: &Value) -> String {
    let path = result
        .get("path")
        .and_then(Value::as_str)
        .or_else(|| input.get("path").and_then(Value::as_str))
        .unwrap_or("<unknown>");
    let replacements = result
        .get("replacements")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let line_delta = result
        .get("line_delta")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    let old_string = input
        .get("old_string")
        .and_then(Value::as_str)
        .or_else(|| result.get("old_preview").and_then(Value::as_str))
        .unwrap_or_default();
    let new_string = input
        .get("new_string")
        .and_then(Value::as_str)
        .or_else(|| result.get("new_preview").and_then(Value::as_str))
        .unwrap_or_default();
    let diff = simple_patch_diff(path, old_string, new_string);
    format!("replace {path}\nreplacements={replacements} line_delta={line_delta}\ndiff:\n{diff}")
}

fn compact_replace_lines(input: &Value, result: &Value) -> String {
    let path = result
        .get("path")
        .and_then(Value::as_str)
        .or_else(|| input.get("path").and_then(Value::as_str))
        .unwrap_or("<unknown>");
    let start_line = result
        .get("start_line")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let end_line = result
        .get("end_line")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let removed_lines = result
        .get("removed_lines")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let inserted_lines = result
        .get("inserted_lines")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let line_delta = result
        .get("line_delta")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    let old_string = result
        .get("removed_string")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let new_string = input
        .get("new_string")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let diff = simple_patch_diff(path, old_string, new_string);
    format!(
        "replace_lines {path}:{start_line}-{end_line}\nremoved={removed_lines} inserted={inserted_lines} line_delta={line_delta}\ndiff:\n{diff}"
    )
}

fn simple_patch_diff(path: &str, old_string: &str, new_string: &str) -> String {
    let mut lines = vec![
        "*** Begin Patch".to_string(),
        format!("*** Update File: {path}"),
        "@@".to_string(),
    ];
    lines.extend(old_string.lines().map(|line| format!("-{line}")));
    lines.extend(new_string.lines().map(|line| format!("+{line}")));
    lines.push("*** End Patch".to_string());
    lines.join("\n")
}

fn compact_subagent_result(tool_name: &str, result: &Value) -> String {
    let summary = result
        .get("summary")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("Sub-agent finished.");
    let mut rendered = match tool_name {
        "spawn_agent" => {
            let name = result
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("worker");
            format!("spawn_agent {name}: {summary}")
        }
        "explore_agent" => format!("explore_agent {summary}"),
        "plan_agent" => format!("plan_agent {summary}"),
        _ => format!("{tool_name} {summary}"),
    };

    append_request_user_input(&mut rendered, result.get("request_user_input"));
    rendered
}

fn append_request_user_input(rendered: &mut String, request: Option<&Value>) {
    let Some(request) = request else {
        return;
    };
    if request.is_null() {
        return;
    }
    if let Some(question) = request
        .get("question")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        rendered.push_str(&format!("\nrequest_user_input: {question}"));
    }
    if let Some(options) = request.get("options").and_then(Value::as_array) {
        for option in options {
            let Some((label, description)) = parse_request_option(option) else {
                continue;
            };
            rendered.push_str(&format!("\noption: {label} | {description}"));
        }
    }
    if let Some(note) = request
        .get("note")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        rendered.push_str(&format!("\nnote: {note}"));
    }
}

fn parse_request_option(option: &Value) -> Option<(String, String)> {
    if let Some(pair) = option.as_array() {
        let label = pair.first()?.as_str()?.trim();
        let description = pair.get(1).and_then(Value::as_str).unwrap_or("").trim();
        return Some((label.to_string(), description.to_string()));
    }
    if let Some(object) = option.as_object() {
        let label = object
            .get("label")
            .or_else(|| object.get("name"))
            .and_then(Value::as_str)?
            .trim();
        let description = object
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        return Some((label.to_string(), description.to_string()));
    }
    None
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
                .or_else(|| result.get("observed_lines").and_then(Value::as_u64))
                .unwrap_or_default();
            let total_lines_exact = result
                .get("total_lines_exact")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let start_line = result
                .get("start_line")
                .and_then(Value::as_u64)
                .unwrap_or(1);
            let end_line = result
                .get("end_line")
                .and_then(Value::as_u64)
                .unwrap_or(total_lines);
            let truncated = result
                .get("truncated")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let line_truncated = result
                .get("line_truncated")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let next_offset = result.get("next_offset").and_then(Value::as_u64);
            let continuation = match (next_offset, line_truncated) {
                (Some(next_offset), true) => {
                    format!(
                        " Truncated line(s); continue with offset={next_offset} for more lines."
                    )
                }
                (Some(next_offset), false) => {
                    format!(" Truncated; continue with offset={next_offset}.")
                }
                (None, true) => " Truncated line(s).".to_string(),
                (None, false) if truncated => " Truncated.".to_string(),
                (None, false) => String::new(),
            };
            let total_label = if total_lines_exact {
                total_lines.to_string()
            } else {
                format!("at least {total_lines}")
            };
            if total_lines > 0 && (start_line != 1 || end_line != total_lines) {
                format!(
                    "Read file {path} lines {start_line}-{end_line} of {total_label} ({total_chars} chars).{continuation}"
                )
            } else {
                format!(
                    "Read file {path} ({total_label} lines, {total_chars} chars).{continuation}"
                )
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
        "web_search" => {
            let query = input
                .get("query")
                .and_then(Value::as_str)
                .or_else(|| result.get("query").and_then(Value::as_str))
                .unwrap_or("<unknown>");
            let total_chars = result
                .get("content")
                .and_then(Value::as_str)
                .map(|content| content.chars().count())
                .unwrap_or_default();
            format!("Searched web for {query:?} ({total_chars} chars).")
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
        "replace_lines" => {
            let path = result
                .get("path")
                .and_then(Value::as_str)
                .or_else(|| input.get("path").and_then(Value::as_str))
                .unwrap_or("<unknown>");
            let start_line = result
                .get("start_line")
                .and_then(Value::as_u64)
                .or_else(|| input.get("start_line").and_then(Value::as_u64))
                .unwrap_or_default();
            let end_line = result
                .get("end_line")
                .and_then(Value::as_u64)
                .or_else(|| input.get("end_line").and_then(Value::as_u64))
                .unwrap_or_default();
            let inserted_lines = result
                .get("inserted_lines")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            format!(
                "Replaced lines {start_line}-{end_line} in {path}: {inserted_lines} inserted line(s)."
            )
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
        ToolResultStore, compact_read_file, compact_subagent_result, compact_web_search,
        default_tool_result_store_dir, repair_tool_result_history,
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
    fn read_file_summary_distinguishes_line_truncation_from_more_lines() {
        let summary = compact_read_file(
            &json!({ "path": "src/generated.json" }),
            &json!({
                "content": "x".repeat(4_020),
                "total_lines": 1,
                "total_lines_exact": true,
                "start_line": 1,
                "end_line": 1,
                "truncated": true,
                "line_truncated": true,
                "next_offset": null,
            }),
        );

        assert!(summary.contains("Read file src/generated.json"));
        assert!(summary.contains("Truncated line(s)."));
        assert!(!summary.contains("continue with offset"));
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
        assert!(
            default_tool_result_store_dir()
                .expect("default tool result dir")
                .ends_with(std::path::Path::new("tool-results"))
        );
    }

    #[test]
    fn compacts_bash_results_with_exit_code_duration_and_aggregated_output() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let store = ToolResultStore::new(tempdir.path()).expect("store");
        let output = store
            .compact_result(
                "bash",
                "tool-bash",
                &json!({ "program": "cargo", "args": ["check"] }),
                &json!({
                    "stdout": "stdout-only\n",
                    "stderr": "stderr-only\n",
                    "aggregated_output": "checking\n[stderr] warning\n",
                    "exit_code": 101,
                    "duration_ms": 1234,
                    "live_streamed": true,
                    "sandboxed": true,
                    "sandbox_backend": "macos-seatbelt"
                }),
            )
            .expect("compact bash result");

        assert!(output.contains("bash finished."));
        assert!(output.contains("Exit code: 101"));
        assert!(output.contains("Duration: 1234 ms"));
        assert!(output.contains("Output:\nchecking\n[stderr] warning"));
        assert!(!output.contains("stdout-only"));
        assert!(!output.contains("stderr-only"));
    }

    #[test]
    fn compacts_bash_fallback_separates_stdout_and_stderr() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let store = ToolResultStore::new(tempdir.path()).expect("store");
        let output = store
            .compact_result(
                "bash",
                "tool-bash",
                &json!({ "program": "sh" }),
                &json!({
                    "stdout": "stdout-without-newline",
                    "stderr": "stderr-line\n",
                    "exit_code": 1,
                    "duration_ms": 10,
                }),
            )
            .expect("compact bash result");

        assert!(output.contains("stdout-without-newline\n[stderr] stderr-line"));
    }

    #[test]
    fn compacts_background_bash_with_task_id() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let store = ToolResultStore::new(tempdir.path()).expect("store");
        let output = store
            .compact_result(
                "bash",
                "tool-bash",
                &json!({ "program": "sh", "run_in_background": true }),
                &json!({
                    "background_task_id": "bash-123",
                    "status": "running",
                    "output_path": "/tmp/rara/bash-123.log",
                    "exit_code": null,
                    "stdout": "",
                    "stderr": "",
                }),
            )
            .expect("compact background bash result");

        assert!(output.contains("bash started in background."));
        assert!(output.contains("Task id: bash-123"));
        assert!(output.contains("Status: running"));
        assert!(output.contains("background_task_status"));
    }

    #[test]
    fn replace_results_include_renderable_diff_preview() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let store = ToolResultStore::new(tempdir.path()).expect("store");
        let output = store
            .compact_result(
                "replace",
                "tool-1",
                &json!({
                    "path": "src/main.rs",
                    "old_string": "let old = true;",
                    "new_string": "let new = true;"
                }),
                &json!({
                    "status": "ok",
                    "path": "src/main.rs",
                    "replacements": 1,
                    "line_delta": 0
                }),
            )
            .expect("compact replace");

        assert!(output.contains("diff:"));
        assert!(output.contains("*** Update File: src/main.rs"));
        assert!(output.contains("-let old = true;"));
        assert!(output.contains("+let new = true;"));
    }

    #[test]
    fn compacts_web_search_with_preview() {
        let output = compact_web_search(
            &json!({ "query": "rara exa mcp" }),
            &json!({
                "query": "rara exa mcp",
                "provider": "exa_mcp",
                "content": "Result one\nResult two"
            }),
        );

        assert!(output.contains("Searched web for \"rara exa mcp\""));
        assert!(output.contains("Results:"));
        assert!(output.contains("Result one"));
    }

    #[test]
    fn compacts_subagent_results_without_full_payload() {
        let compacted = compact_subagent_result(
            "spawn_agent",
            &json!({
                "name": "fix-assembler",
                "status": "done",
                "summary": "Removed the orphaned test block and kept one cfg(test) module.",
                "request_user_input": {
                    "question": "Proceed?",
                    "options": [
                        ["Yes", "Apply the cleanup."],
                        { "label": "No", "description": "Leave the file unchanged." }
                    ],
                    "note": "The line range was verified."
                }
            }),
        );

        assert!(compacted.starts_with("spawn_agent fix-assembler: Removed"));
        assert!(compacted.contains("request_user_input: Proceed?"));
        assert!(compacted.contains("option: Yes | Apply the cleanup."));
        assert!(compacted.contains("option: No | Leave the file unchanged."));
        assert!(compacted.contains("note: The line range was verified."));
        assert!(!compacted.contains("\"summary\""));
        assert!(!compacted.contains("\"request_user_input\""));
    }
}
