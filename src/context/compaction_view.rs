use serde_json::Value;

use crate::agent::Message;
use crate::context::CompactionSourceContextEntry;

pub(crate) fn compaction_source_entries(history: &[Message]) -> Vec<CompactionSourceContextEntry> {
    let mut entries = Vec::new();
    let mut compact_boundary_seen = false;

    for message in history {
        let Some(items) = message.content.as_array() else {
            continue;
        };
        for item in items {
            let Some(item_type) = item.get("type").and_then(Value::as_str) else {
                continue;
            };
            match item_type {
                "compacted_summary" => entries.push(CompactionSourceContextEntry {
                    order: 0,
                    kind: "compacted_summary".to_string(),
                    label: "Compacted Summary".to_string(),
                    detail: summarize_text_block(item.get("text").and_then(Value::as_str)),
                    inclusion_reason: "carried forward because the conversation history was compacted into a summary block".to_string(),
                }),
                "recent_files" => entries.push(CompactionSourceContextEntry {
                    order: 0,
                    kind: "recent_files".to_string(),
                    label: "Recent Files".to_string(),
                    detail: summarize_recent_files(item.get("files").and_then(Value::as_array)),
                    inclusion_reason: "carried forward so the next turn keeps a lightweight view of recently touched files".to_string(),
                }),
                "recent_file_excerpts" => entries.push(CompactionSourceContextEntry {
                    order: 0,
                    kind: "recent_file_excerpts".to_string(),
                    label: "Recent File Excerpts".to_string(),
                    detail: summarize_recent_file_excerpts(item.get("files").and_then(Value::as_array)),
                    inclusion_reason: "carried forward so the next turn retains short excerpts from recently referenced files".to_string(),
                }),
                "compact_boundary" if !compact_boundary_seen => {
                    compact_boundary_seen = true;
                    entries.push(CompactionSourceContextEntry {
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
        .unwrap_or_else(|| "no summary text".to_string())
}

fn summarize_recent_files(files: Option<&Vec<Value>>) -> String {
    let files = files
        .into_iter()
        .flat_map(|items| items.iter())
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    match files.len() {
        0 => "no files".to_string(),
        1 => files[0].to_string(),
        _ => format!("{} (+{} more)", files[0], files.len() - 1),
    }
}

fn summarize_recent_file_excerpts(files: Option<&Vec<Value>>) -> String {
    let count = files.into_iter().flat_map(|items| items.iter()).count();
    match count {
        0 => "no excerpts".to_string(),
        1 => "1 excerpt".to_string(),
        _ => format!("{count} excerpts"),
    }
}

fn summarize_compact_boundary(item: &Value) -> String {
    let version = item
        .get("version")
        .and_then(Value::as_u64)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string());
    let before_tokens = item
        .get("before_tokens")
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
