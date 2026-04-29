use crate::agent::Message;
use serde_json::Value;

/// Extract structured compaction metadata from history messages.
pub(crate) fn compaction_source_entries(history: &[Message]) -> Vec<CompactionSourceItem> {
    let mut entries = Vec::new();
    let mut compact_boundary_seen = false;

    for message in history.iter().rev() {
        let Some(blocks) = message.content.as_array() else {
            continue;
        };
        for item in blocks {
            let Some(item_type) = item.get("type").and_then(Value::as_str) else {
                continue;
            };
            match item_type {
                "compacted_summary" => entries.push(CompactionSourceItem {
                    order: 0,
                    kind: "compacted_summary".to_string(),
                    label: "Compacted Summary".to_string(),
                    detail: summarize_text_block(item.get("text").and_then(Value::as_str)),
                    inclusion_reason: "carried forward because the conversation history was compacted into a summary block".to_string(),
                }),
                "recent_files" => entries.push(CompactionSourceItem {
                    order: 0,
                    kind: "recent_files".to_string(),
                    label: "Recent Files".to_string(),
                    detail: summarize_recent_files(item.get("files").and_then(Value::as_array)),
                    inclusion_reason: "carried forward so the next turn keeps a lightweight view of recently touched files".to_string(),
                }),
                "recent_file_excerpts" => entries.push(CompactionSourceItem {
                    order: 0,
                    kind: "recent_file_excerpts".to_string(),
                    label: "Recent File Excerpts".to_string(),
                    detail: summarize_recent_file_excerpts(item.get("files").and_then(Value::as_array)),
                    inclusion_reason: "carried forward so the next turn retains short excerpts from recently referenced files".to_string(),
                }),
                "compact_boundary" if !compact_boundary_seen => {
                    compact_boundary_seen = true;
                    entries.push(CompactionSourceItem {
                        order: 0,
                        kind: "compact_boundary".to_string(),
                        label: "Compaction Boundary".to_string(),
                        detail: summarize_compact_boundary(item),
                        inclusion_reason: "recorded to explain where the latest compaction boundary cut the thread history".to_string(),
                    });
                }
                _ => {}
            }
        }
    }

    for (idx, entry) in entries.iter_mut().enumerate() {
        entry.order = idx + 1;
    }
    entries
}

#[derive(Debug, Clone)]
pub(crate) struct CompactionSourceItem {
    pub(crate) order: usize,
    pub(crate) kind: String,
    pub(crate) label: String,
    pub(crate) detail: String,
    pub(crate) inclusion_reason: String,
}

pub(crate) fn summarize_workspace_memory_source(content: &str) -> String {
    let line_count = content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
    match line_count {
        0 => "empty".to_string(),
        1 => "1 non-empty line".to_string(),
        _ => format!("{line_count} non-empty lines"),
    }
}

pub(crate) fn extract_json_array_strings(content: &str, key: &str) -> Vec<String> {
    extract_tool_result_payload(content)
        .and_then(|payload| payload.get(key).and_then(Value::as_array).cloned())
        .into_iter()
        .flatten()
        .filter_map(|item| item.as_str().map(str::trim).map(str::to_string))
        .filter(|value| !value.is_empty())
        .collect()
}

pub(crate) fn extract_json_string_field(content: &str, key: &str) -> Option<String> {
    extract_tool_result_payload(content)
        .and_then(|payload| payload.get(key).and_then(Value::as_str).map(str::to_string))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(crate) fn extract_tool_result_payload(content: &str) -> Option<Value> {
    let payload = content
        .split_once("Payload:\n")
        .map(|(_, payload)| payload)
        .unwrap_or(content)
        .trim();
    serde_json::from_str(payload).ok()
}

fn summarize_text_block(text: Option<&str>) -> String {
    text.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            let condensed = value.split_whitespace().collect::<Vec<_>>().join(" ");
            if condensed.len() > 96 {
                format!("{}...", &condensed[..93])
            } else {
                condensed
            }
        })
        .unwrap_or_else(|| "-".to_string())
}

fn summarize_recent_files(files: Option<&Vec<Value>>) -> String {
    let paths = files
        .into_iter()
        .flatten()
        .filter_map(|item| item.as_str())
        .take(8)
        .collect::<Vec<_>>();
    if paths.is_empty() {
        "-".to_string()
    } else {
        paths.join(", ")
    }
}

fn summarize_recent_file_excerpts(files: Option<&Vec<Value>>) -> String {
    let paths = files
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("path").and_then(Value::as_str))
        .take(4)
        .collect::<Vec<_>>();
    if paths.is_empty() {
        "-".to_string()
    } else {
        format!("excerpts for: {}", paths.join(", "))
    }
}

fn summarize_compact_boundary(item: &Value) -> String {
    let version = item
        .get("compaction_version")
        .and_then(Value::as_u64)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string());
    let before_tokens = item
        .get("estimated_tokens_before")
        .and_then(Value::as_u64)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string());
    let recent_file_count = item
        .get("recent_file_count")
        .and_then(Value::as_u64)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string());
    format!("version={version} before_tokens={before_tokens} recent_files={recent_file_count}")
}
