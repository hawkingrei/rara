use super::*;
use crate::llm::ContextBudget;

const RECENT_FILE_CARRY_OVER_LIMIT: usize = 5;
const RECENT_FILE_EXCERPT_LIMIT: usize = 3;
const RECENT_FILE_EXCERPT_CHAR_LIMIT: usize = 600;
const COMPACT_BOUNDARY_KIND: &str = "compact_boundary";
const COMPACT_BOUNDARY_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompactBoundaryMetadata {
    pub version: u32,
    pub before_tokens: usize,
    pub recent_file_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecentFileExcerpt {
    path: String,
    line_range: Option<(usize, usize)>,
    snippet: String,
}

#[derive(Debug, Clone, Default)]
pub struct CompactState {
    pub estimated_history_tokens: usize,
    pub context_window_tokens: Option<usize>,
    pub compact_threshold_tokens: usize,
    pub reserved_output_tokens: usize,
    pub compaction_count: usize,
    pub last_compaction_before_tokens: Option<usize>,
    pub last_compaction_after_tokens: Option<usize>,
    pub last_compaction_recent_files: Vec<String>,
    pub last_compaction_boundary: Option<CompactBoundaryMetadata>,
}

impl Agent {
    pub async fn compact_if_needed(&mut self) -> Result<()> {
        self.compact_if_needed_with_reporter(|_| {}).await
    }

    pub async fn compact_if_needed_with_reporter<F>(&mut self, mut report: F) -> Result<()>
    where
        F: FnMut(AgentEvent) + Send,
    {
        self.compact_history_with_reporter(false, &mut report).await
    }

    pub async fn compact_now_with_reporter<F>(&mut self, mut report: F) -> Result<bool>
    where
        F: FnMut(AgentEvent) + Send,
    {
        self.compact_history_with_reporter(true, &mut report)
            .await?;
        Ok(self.compact_state.last_compaction_before_tokens.is_some())
    }

    async fn compact_history_with_reporter<F>(&mut self, force: bool, report: &mut F) -> Result<()>
    where
        F: FnMut(AgentEvent) + Send,
    {
        let current_tokens = estimate_history_tokens(&self.history)?;
        let compact_budget = self.current_compact_budget();
        self.compact_state.estimated_history_tokens = current_tokens;
        self.compact_state.context_window_tokens = compact_budget
            .as_ref()
            .map(|budget| budget.context_window_tokens);
        self.compact_state.compact_threshold_tokens = compact_budget
            .as_ref()
            .map(|budget| budget.compact_threshold_tokens)
            .unwrap_or(10_000);
        self.compact_state.reserved_output_tokens = compact_budget
            .as_ref()
            .map(|budget| budget.reserved_output_tokens)
            .unwrap_or(0);
        self.compact_state.last_compaction_before_tokens = None;
        self.compact_state.last_compaction_after_tokens = None;
        self.compact_state.last_compaction_recent_files.clear();
        self.compact_state.last_compaction_boundary = None;

        let threshold = self.compact_state.compact_threshold_tokens;
        if !force && current_tokens <= threshold {
            return Ok(());
        }
        if self.history.len() < 2 {
            return Ok(());
        }

        report(AgentEvent::Status(if force {
            "Compacting conversation history on demand.".to_string()
        } else {
            "Compacting long conversation history.".to_string()
        }));

        let split_idx = (self.history.len() as f64 * 0.8) as usize;
        let split_idx = split_idx.clamp(1, self.history.len().saturating_sub(1));
        let summary = self
            .llm_backend
            .summarize(
                &self.history[..split_idx],
                &prompt::build_compact_instruction(&self.prompt_config),
            )
            .await?;
        let recent_files =
            collect_recent_files(&self.history[..split_idx], RECENT_FILE_CARRY_OVER_LIMIT);
        let recent_file_excerpts = collect_recent_file_excerpts(
            &self.history[..split_idx],
            RECENT_FILE_EXCERPT_LIMIT,
            RECENT_FILE_EXCERPT_CHAR_LIMIT,
        );
        let mut new_history = vec![
            build_compact_boundary_message(current_tokens, recent_files.len()),
            Message {
                role: "system".to_string(),
                content: json!(format!(
                    "STRUCTURED SUMMARY OF PREVIOUS CONVERSATION:\n{}",
                    summary
                )),
            },
        ];
        if !recent_files.is_empty() {
            let recent_files_block = recent_files
                .iter()
                .map(|path| format!("- {path}"))
                .collect::<Vec<_>>()
                .join("\n");
            new_history.push(Message {
                role: "system".to_string(),
                content: json!(format!(
                    "RECENT FILES FROM COMPACTED HISTORY:\n{}",
                    recent_files_block
                )),
            });
        }
        if !recent_file_excerpts.is_empty() {
            let excerpt_block = recent_file_excerpts
                .iter()
                .map(render_recent_file_excerpt)
                .collect::<Vec<_>>()
                .join("\n\n");
            new_history.push(Message {
                role: "system".to_string(),
                content: json!(format!(
                    "RECENT FILE EXCERPTS FROM COMPACTED HISTORY:\n{}",
                    excerpt_block
                )),
            });
        }
        new_history.extend_from_slice(&self.history[split_idx..]);
        self.replace_history(new_history);
        self.session_manager
            .save_session(&self.session_id, &self.history)?;

        let compacted_tokens = self.compact_state.estimated_history_tokens;
        self.compact_state.compaction_count += 1;
        self.compact_state.last_compaction_before_tokens = Some(current_tokens);
        self.compact_state.last_compaction_after_tokens = Some(compacted_tokens);
        self.compact_state.last_compaction_recent_files = recent_files;
        self.compact_state.last_compaction_boundary = Some(CompactBoundaryMetadata {
            version: COMPACT_BOUNDARY_VERSION,
            before_tokens: current_tokens,
            recent_file_count: self.compact_state.last_compaction_recent_files.len(),
        });
        Ok(())
    }

    pub(super) fn current_compact_budget(&self) -> Option<ContextBudget> {
        let tools = self.visible_tool_schemas();
        self.llm_backend.context_budget(&self.history, &tools)
    }

    pub(super) fn push_history_message(&mut self, message: Message) {
        self.record_history_message_tokens(&message);
        self.history.push(message);
    }

    pub(super) fn extend_history_messages(&mut self, messages: Vec<Message>) {
        self.record_history_messages_tokens(&messages);
        self.history.extend(messages);
    }

    pub(super) fn replace_history(&mut self, history: Vec<Message>) {
        self.history = history;
        self.recompute_history_token_estimate();
    }

    fn record_history_message_tokens(&mut self, message: &Message) {
        if let Ok(tokens) = estimate_message_tokens(message) {
            self.compact_state.estimated_history_tokens += tokens;
        } else {
            self.recompute_history_token_estimate();
        }
    }

    fn record_history_messages_tokens(&mut self, messages: &[Message]) {
        let mut total = 0usize;
        for message in messages {
            match estimate_message_tokens(message) {
                Ok(tokens) => total += tokens,
                Err(_) => {
                    self.recompute_history_token_estimate();
                    return;
                }
            }
        }
        self.compact_state.estimated_history_tokens += total;
    }

    fn recompute_history_token_estimate(&mut self) {
        self.compact_state.estimated_history_tokens =
            estimate_history_tokens(&self.history).unwrap_or_default();
    }
}

fn estimate_history_tokens(history: &[Message]) -> Result<usize> {
    let bpe = tiktoken_rs::cl100k_base()?;
    Ok(history
        .iter()
        .map(|message| estimate_message_tokens_with_bpe(message, &bpe))
        .sum::<Result<usize>>()?)
}

fn estimate_message_tokens(message: &Message) -> Result<usize> {
    let bpe = tiktoken_rs::cl100k_base()?;
    estimate_message_tokens_with_bpe(message, &bpe)
}

fn estimate_message_tokens_with_bpe(
    message: &Message,
    bpe: &tiktoken_rs::CoreBPE,
) -> Result<usize> {
    Ok(bpe
        .encode_with_special_tokens(&message.content.to_string())
        .len())
}

fn collect_recent_files(history: &[Message], limit: usize) -> Vec<String> {
    let mut collected = Vec::new();
    for message in history.iter().rev() {
        if message.role != "assistant" {
            continue;
        }
        let Some(items) = message.content.as_array() else {
            continue;
        };
        for item in items.iter().rev() {
            if item.get("type").and_then(Value::as_str) != Some("tool_use") {
                continue;
            }
            let Some(tool_name) = item.get("name").and_then(Value::as_str) else {
                continue;
            };
            let Some(input) = item.get("input").and_then(Value::as_object) else {
                continue;
            };
            let Some(path) = input.get("path").and_then(Value::as_str) else {
                continue;
            };
            if !matches!(
                tool_name,
                "read_file"
                    | "list_files"
                    | "write_file"
                    | "replace"
                    | "apply_patch"
            ) {
                continue;
            }
            let normalized = path.replace('\\', "/");
            if !collected.iter().any(|existing| existing == &normalized) {
                collected.push(normalized);
            }
            if collected.len() >= limit {
                return collected;
            }
        }
    }
    collected
}

fn collect_recent_file_excerpts(
    history: &[Message],
    limit: usize,
    char_limit: usize,
) -> Vec<RecentFileExcerpt> {
    use std::collections::HashMap;

    let mut pending_reads = HashMap::<String, (String, Option<(usize, usize)>)>::new();
    let mut excerpts = Vec::new();

    for message in history {
        match message.role.as_str() {
            "assistant" => {
                let Some(items) = message.content.as_array() else {
                    continue;
                };
                for item in items {
                    if item.get("type").and_then(Value::as_str) != Some("tool_use") {
                        continue;
                    }
                    if item.get("name").and_then(Value::as_str) != Some("read_file") {
                        continue;
                    }
                    let Some(tool_use_id) = item.get("id").and_then(Value::as_str) else {
                        continue;
                    };
                    let Some(input) = item.get("input").and_then(Value::as_object) else {
                        continue;
                    };
                    let Some(path) = input.get("path").and_then(Value::as_str) else {
                        continue;
                    };
                    let line_range = match (
                        input.get("start_line").and_then(Value::as_u64),
                        input.get("end_line").and_then(Value::as_u64),
                    ) {
                        (Some(start), Some(end)) => Some((start as usize, end as usize)),
                        (Some(start), None) => Some((start as usize, start as usize)),
                        _ => None,
                    };
                    pending_reads.insert(
                        tool_use_id.to_string(),
                        (path.replace('\\', "/"), line_range),
                    );
                }
            }
            "user" => {
                let Some(items) = message.content.as_array() else {
                    continue;
                };
                for item in items {
                    if item.get("type").and_then(Value::as_str) != Some("tool_result") {
                        continue;
                    }
                    let Some(tool_use_id) = item.get("tool_use_id").and_then(Value::as_str) else {
                        continue;
                    };
                    let Some((path, line_range)) = pending_reads.remove(tool_use_id) else {
                        continue;
                    };
                    let snippet = item
                        .get("content")
                        .and_then(Value::as_str)
                        .map(|content| truncate_excerpt(content, char_limit).trim().to_string())
                        .filter(|content| !content.is_empty());
                    let Some(snippet) = snippet else {
                        continue;
                    };
                    excerpts.retain(|existing: &RecentFileExcerpt| existing.path != path);
                    excerpts.push(RecentFileExcerpt {
                        path,
                        line_range,
                        snippet,
                    });
                }
            }
            _ => {}
        }
    }

    if excerpts.len() > limit {
        excerpts = excerpts[excerpts.len() - limit..].to_vec();
    }
    excerpts.reverse();
    excerpts
}

fn render_recent_file_excerpt(excerpt: &RecentFileExcerpt) -> String {
    let header = match excerpt.line_range {
        Some((start, end)) if start != end => {
            format!("### {} (lines {}-{})", excerpt.path, start, end)
        }
        Some((line, _)) => format!("### {} (line {})", excerpt.path, line),
        None => format!("### {}", excerpt.path),
    };
    format!("{header}\n```text\n{}\n```", excerpt.snippet)
}

fn truncate_excerpt(text: &str, max_chars: usize) -> String {
    let total = text.chars().count();
    if total <= max_chars {
        return text.to_string();
    }
    let truncated = text.chars().take(max_chars).collect::<String>();
    format!("{truncated}\n... truncated.")
}

fn build_compact_boundary_message(before_tokens: usize, recent_file_count: usize) -> Message {
    Message {
        role: "system".to_string(),
        content: json!({
            "type": COMPACT_BOUNDARY_KIND,
            "version": COMPACT_BOUNDARY_VERSION,
            "before_tokens": before_tokens,
            "recent_file_count": recent_file_count,
        }),
    }
}

pub fn latest_compact_boundary_metadata(history: &[Message]) -> Option<CompactBoundaryMetadata> {
    history.iter().rev().find_map(|message| {
        let content = message.content.as_object()?;
        if content.get("type").and_then(Value::as_str) != Some(COMPACT_BOUNDARY_KIND) {
            return None;
        }
        Some(CompactBoundaryMetadata {
            version: content
                .get("version")
                .and_then(Value::as_u64)
                .unwrap_or(COMPACT_BOUNDARY_VERSION as u64) as u32,
            before_tokens: content
                .get("before_tokens")
                .and_then(Value::as_u64)
                .unwrap_or_default() as usize,
            recent_file_count: content
                .get("recent_file_count")
                .and_then(Value::as_u64)
                .unwrap_or_default() as usize,
        })
    })
}
