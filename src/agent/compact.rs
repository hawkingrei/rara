use super::*;
use crate::llm::ContextBudget;
use crate::session::PersistedCompactionEvent;
use std::sync::OnceLock;
use std::time::Duration;

const RECENT_FILE_CARRY_OVER_LIMIT: usize = 5;
const RECENT_FILE_EXCERPT_LIMIT: usize = 3;
const RECENT_FILE_EXCERPT_CHAR_LIMIT: usize = 600;
const COMPACT_BOUNDARY_KIND: &str = "compact_boundary";
const COMPACT_BOUNDARY_VERSION: u32 = 1;
// Wait for about two 4K chunks of new context before retrying automatic
// compaction after a timeout or backend failure.
const AUTO_COMPACTION_RETRY_HYSTERESIS_TOKENS: usize = 8_192;
#[cfg(not(test))]
const COMPACTION_SUMMARY_TIMEOUT: Duration = Duration::from_secs(120);

#[cfg(test)]
const TEST_COMPACTION_SUMMARY_TIMEOUT: Duration = Duration::from_millis(10);

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
    pub consecutive_auto_compaction_failures: usize,
    pub auto_compaction_retry_after_tokens: Option<usize>,
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
        if !force
            && self
                .compact_state
                .auto_compaction_retry_after_tokens
                .is_some_and(|retry_after| current_tokens < retry_after)
        {
            report(AgentEvent::Status(
                "Automatic history compaction is temporarily suspended after a previous failure."
                    .to_string(),
            ));
            return Ok(());
        }

        report(AgentEvent::Status(if force {
            "Compacting conversation history on demand.".to_string()
        } else {
            "Compacting long conversation history.".to_string()
        }));

        let split_idx = (self.history.len() as f64 * 0.8) as usize;
        let split_idx = split_idx.clamp(1, self.history.len().saturating_sub(1));
        let summary_result = tokio::time::timeout(
            compaction_summary_timeout(),
            self.llm_backend.summarize(
                &self.history[..split_idx],
                &self.context_assembler().compact_instruction(),
            ),
        )
        .await;
        let summary = match summary_result {
            Ok(summary) => match summary {
                Ok(summary) => summary,
                Err(err) if !force => {
                    self.record_auto_compaction_failure(current_tokens);
                    report(AgentEvent::Status(format!(
                        "Automatic history compaction failed; continuing without compaction. {err}"
                    )));
                    return Ok(());
                }
                Err(err) => return Err(err),
            },
            Err(_) if !force => {
                self.record_auto_compaction_failure(current_tokens);
                report(AgentEvent::Status(
                    "Automatic history compaction timed out; continuing without compaction."
                        .to_string(),
                ));
                return Ok(());
            }
            Err(_) => {
                return Err(anyhow::anyhow!(
                    "history compaction timed out after {} seconds",
                    compaction_summary_timeout().as_secs()
                ));
            }
        };
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
        self.compact_state.consecutive_auto_compaction_failures = 0;
        self.compact_state.auto_compaction_retry_after_tokens = None;
        self.session_manager.save_compaction_event(
            &self.session_id,
            &PersistedCompactionEvent {
                event_index: self.compact_state.compaction_count,
                before_tokens: current_tokens,
                after_tokens: compacted_tokens,
                boundary_version: COMPACT_BOUNDARY_VERSION,
                recent_files: self.compact_state.last_compaction_recent_files.clone(),
                summary,
            },
        )?;
        Ok(())
    }

    fn record_auto_compaction_failure(&mut self, current_tokens: usize) {
        self.compact_state.consecutive_auto_compaction_failures += 1;
        self.compact_state.auto_compaction_retry_after_tokens =
            Some(current_tokens.saturating_add(AUTO_COMPACTION_RETRY_HYSTERESIS_TOKENS));
    }

    pub(super) fn current_compact_budget(&self) -> Option<ContextBudget> {
        let tools = self.visible_tool_schemas();
        self.context_assembler()
            .budget_for(self.llm_backend.as_ref(), &self.history, &tools)
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

fn compaction_summary_timeout() -> Duration {
    #[cfg(test)]
    {
        TEST_COMPACTION_SUMMARY_TIMEOUT
    }
    #[cfg(not(test))]
    {
        COMPACTION_SUMMARY_TIMEOUT
    }
}

fn estimate_history_tokens(history: &[Message]) -> Result<usize> {
    let bpe = tokenizer()?;
    history
        .iter()
        .map(|message| estimate_message_tokens_with_bpe(message, bpe))
        .sum::<Result<usize>>()
}

fn estimate_message_tokens(message: &Message) -> Result<usize> {
    let bpe = tokenizer()?;
    estimate_message_tokens_with_bpe(message, bpe)
}

fn estimate_message_tokens_with_bpe(
    message: &Message,
    bpe: &tiktoken_rs::CoreBPE,
) -> Result<usize> {
    let rendered;
    let content = if let Some(text) = message.content.as_str() {
        text
    } else {
        rendered = message.content.to_string();
        rendered.as_str()
    };
    Ok(bpe.encode_with_special_tokens(content).len())
}

fn tokenizer() -> Result<&'static tiktoken_rs::CoreBPE> {
    static BPE: OnceLock<std::result::Result<tiktoken_rs::CoreBPE, String>> = OnceLock::new();
    match BPE.get_or_init(|| tiktoken_rs::cl100k_base().map_err(|err| err.to_string())) {
        Ok(bpe) => Ok(bpe),
        Err(err) => Err(anyhow::anyhow!(err.clone())),
    }
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
                "read_file" | "list_files" | "write_file" | "replace" | "apply_patch"
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
                    let line_range = read_file_line_range(input);
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

fn read_file_line_range(input: &serde_json::Map<String, Value>) -> Option<(usize, usize)> {
    match (
        input.get("offset").and_then(Value::as_u64),
        input.get("limit").and_then(Value::as_u64),
    ) {
        (Some(offset), Some(limit)) if limit > 0 => {
            let start = usize::try_from(offset).ok()?;
            let limit = usize::try_from(limit).ok()?;
            let end = start.checked_add(limit)?.checked_sub(1)?;
            return Some((start, end));
        }
        (Some(offset), None) => {
            let start = usize::try_from(offset).ok()?;
            return Some((start, start));
        }
        _ => {}
    }

    match (
        input.get("start_line").and_then(Value::as_u64),
        input.get("end_line").and_then(Value::as_u64),
    ) {
        (Some(start), Some(end)) => {
            let start = usize::try_from(start).ok()?;
            let end = usize::try_from(end).ok()?;
            Some((start, end))
        }
        (Some(start), None) => {
            let start = usize::try_from(start).ok()?;
            Some((start, start))
        }
        _ => None,
    }
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

#[cfg(test)]
mod tests {
    use super::read_file_line_range;
    use serde_json::{Map, Value, json};

    fn object(value: Value) -> Map<String, Value> {
        value.as_object().expect("object").clone()
    }

    #[test]
    fn read_file_line_range_rejects_overflowing_offset_limit() {
        let input = object(json!({
            "offset": usize::MAX,
            "limit": 2,
        }));

        assert_eq!(read_file_line_range(&input), None);
    }

    #[test]
    fn read_file_line_range_accepts_checked_offset_limit() {
        let input = object(json!({
            "offset": 10,
            "limit": 3,
        }));

        assert_eq!(read_file_line_range(&input), Some((10, 12)));
    }
}
