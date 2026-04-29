use anyhow::{anyhow, Result};
use async_trait::async_trait;
use futures::{Stream, StreamExt};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;
use url::Url;

use crate::agent::Message;
use crate::llm::{ContentBlock, LlmResponse, TokenUsage};

#[derive(Debug, Clone, PartialEq)]
pub(super) struct AssistantToolUse {
    pub id: String,
    pub name: String,
    pub input: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextBudget {
    pub context_window_tokens: usize,
    pub reserved_output_tokens: usize,
    pub compact_threshold_tokens: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmExecutionMode {
    Execute,
    Plan,
}

#[derive(Debug, Clone)]
pub struct LlmTurnMetadata {
    execution_mode: LlmExecutionMode,
    cancellation: Option<Arc<AtomicBool>>,
}

impl Default for LlmTurnMetadata {
    fn default() -> Self {
        Self {
            execution_mode: LlmExecutionMode::Execute,
            cancellation: None,
        }
    }
}

impl LlmTurnMetadata {
    pub fn execute() -> Self {
        Self {
            execution_mode: LlmExecutionMode::Execute,
            cancellation: None,
        }
    }

    pub fn plan() -> Self {
        Self {
            execution_mode: LlmExecutionMode::Plan,
            cancellation: None,
        }
    }

    pub fn with_cancellation(mut self, cancellation: Arc<AtomicBool>) -> Self {
        self.cancellation = Some(cancellation);
        self
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancellation
            .as_ref()
            .is_some_and(|cancellation| cancellation.load(Ordering::SeqCst))
    }

    pub fn ensure_not_cancelled(&self) -> Result<()> {
        if self.is_cancelled() {
            return Err(anyhow!("LLM turn cancelled by user"));
        }
        Ok(())
    }

    pub fn prefers_strong_reasoning(&self) -> bool {
        matches!(self.execution_mode, LlmExecutionMode::Plan)
    }
}

#[async_trait]
pub trait LlmBackend: Send + Sync {
    async fn ask(&self, messages: &[Message], tools: &[Value]) -> Result<LlmResponse>;
    async fn ask_with_context(
        &self,
        messages: &[Message],
        tools: &[Value],
        _metadata: LlmTurnMetadata,
    ) -> Result<LlmResponse> {
        self.ask(messages, tools).await
    }

    async fn ask_streaming(
        &self,
        messages: &[Message],
        tools: &[Value],
        _on_text_delta: &mut (dyn FnMut(String) + Send),
    ) -> Result<LlmResponse> {
        self.ask(messages, tools).await
    }
    async fn ask_streaming_with_context(
        &self,
        messages: &[Message],
        tools: &[Value],
        _metadata: LlmTurnMetadata,
        on_text_delta: &mut (dyn FnMut(String) + Send),
    ) -> Result<LlmResponse> {
        self.ask_streaming(messages, tools, on_text_delta).await
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>>;
    async fn summarize(&self, messages: &[Message], instruction: &str) -> Result<String>;
    fn context_budget(&self, _messages: &[Message], _tools: &[Value]) -> Option<ContextBudget> {
        None
    }
}

pub struct MockLlm;

#[async_trait]
impl LlmBackend for MockLlm {
    async fn ask(&self, messages: &[Message], _tools: &[Value]) -> Result<LlmResponse> {
        let last_msg_json = &messages.last().unwrap().content;
        let last_msg_text = if last_msg_json.is_array() {
            last_msg_json
                .get(0)
                .and_then(|b| b.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
        } else {
            last_msg_json.as_str().unwrap_or("")
        };
        Ok(LlmResponse {
            content: vec![ContentBlock::Text {
                text: format!("Mock Response: {}", last_msg_text),
            }],
            stop_reason: Some("end_turn".to_string()),
            usage: Some(TokenUsage {
                input_tokens: 10,
                output_tokens: 20,
            }),
        })
    }

    async fn embed(&self, _text: &str) -> Result<Vec<f32>> {
        Ok(vec![0.1; 128])
    }

    async fn summarize(&self, _messages: &[Message], _instruction: &str) -> Result<String> {
        Ok("Mock summary".into())
    }
}

pub(super) fn render_openai_message_content(content: &Value) -> String {
    if let Some(text) = content.as_str() {
        return text.to_string();
    }
    if let Some(items) = content.as_array() {
        let text_parts = items
            .iter()
            .filter_map(|item| match item.get("type").and_then(Value::as_str) {
                Some("text") => item.get("text").and_then(Value::as_str).map(str::to_string),
                Some("tool_result") => item
                    .get("content")
                    .and_then(Value::as_str)
                    .map(|text| format!("tool_result: {text}")),
                _ => None,
            })
            .collect::<Vec<_>>();
        if !text_parts.is_empty() {
            return text_parts.join("\n\n");
        }
    }
    content.to_string()
}

pub(super) fn extract_message_text(content: Option<&Value>) -> Option<String> {
    let content = content?;
    if let Some(text) = content.as_str() {
        return Some(text.to_string());
    }
    let items = content.as_array()?;
    let texts = items
        .iter()
        .filter_map(|item| {
            let item_type = item.get("type").and_then(Value::as_str)?;
            if item_type == "text" {
                item.get("text").and_then(Value::as_str).map(str::to_string)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    if texts.is_empty() {
        None
    } else {
        Some(texts.join("\n\n"))
    }
}

pub(super) fn collect_assistant_content(content: &Value) -> (Vec<String>, Vec<AssistantToolUse>) {
    let mut text_parts = Vec::new();
    let mut tool_uses = Vec::new();

    if let Some(items) = content.as_array() {
        for item in items {
            match item.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                        if !text.trim().is_empty() {
                            text_parts.push(text.to_string());
                        }
                    }
                }
                Some("tool_use") => {
                    tool_uses.push(AssistantToolUse {
                        id: item
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        name: item
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        input: item.get("input").cloned().unwrap_or_else(|| json!({})),
                    });
                }
                _ => {}
            }
        }
    } else if let Some(text) = content.as_str() {
        text_parts.push(text.to_string());
    }

    (text_parts, tool_uses)
}

pub(super) fn extract_single_tool_result(content: &Value) -> Option<(String, String)> {
    let items = content.as_array()?;
    if items.len() != 1 {
        return None;
    }
    let item = &items[0];
    if item.get("type").and_then(Value::as_str) != Some("tool_result") {
        return None;
    }
    Some((
        item.get("tool_use_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        item.get("content")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    ))
}

pub(super) fn parse_tool_arguments(arguments: &Value) -> Result<Value> {
    match arguments {
        Value::String(raw) => serde_json::from_str(raw).map_err(Into::into),
        Value::Object(_) => Ok(arguments.clone()),
        Value::Null => Ok(json!({})),
        _ => Err(anyhow!("tool arguments must be a string or object")),
    }
}

const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const HTTP_READ_TIMEOUT: Duration = Duration::from_secs(300);
const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(120);

pub(super) fn http_client_for_target(base_url: &str) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(HTTP_CONNECT_TIMEOUT)
        .read_timeout(HTTP_READ_TIMEOUT);
    if should_bypass_proxy(base_url) {
        builder = builder.no_proxy();
    }
    builder.build().map_err(Into::into)
}

pub(super) async fn next_stream_item_with_idle_timeout<S, T>(
    stream: &mut S,
    label: &str,
) -> Result<Option<T>>
where
    S: Stream<Item = T> + Unpin,
{
    tokio::time::timeout(STREAM_IDLE_TIMEOUT, stream.next())
        .await
        .map_err(|_| {
            anyhow!(
                "{label} stream produced no events for {} seconds",
                STREAM_IDLE_TIMEOUT.as_secs()
            )
        })
}

pub(super) fn should_bypass_proxy(base_url: &str) -> bool {
    let Ok(url) = Url::parse(base_url) else {
        return false;
    };
    matches!(url.host_str(), Some("localhost") | Some("127.0.0.1"))
}

pub(super) fn context_budget_from_window(context_window_tokens: usize) -> ContextBudget {
    let reserved_output_tokens = (context_window_tokens / 8).clamp(1024, 4096);
    let compact_threshold_tokens = context_window_tokens
        .saturating_sub(reserved_output_tokens)
        .saturating_sub(2048);
    ContextBudget {
        context_window_tokens,
        reserved_output_tokens,
        compact_threshold_tokens,
    }
}

pub(super) fn model_context_budget(model: &str) -> Option<ContextBudget> {
    let canonical = model.trim().to_ascii_lowercase();
    let context_window_tokens = if canonical.contains("deepseek") && canonical.contains("v4") {
        1_000_000
    } else if canonical.contains("gpt-5")
        || canonical.contains("codex")
        || canonical.contains("gpt-4.1")
        || canonical.contains("gpt-4o")
    {
        200_000
    } else if canonical.contains("gpt-4") {
        128_000
    } else {
        return None;
    };
    Some(context_budget_from_window(context_window_tokens))
}

pub(crate) fn hashed_embedding(text: &str, dim: usize) -> Vec<f32> {
    use sha2::{Digest, Sha256};

    let mut values = vec![0f32; dim];
    for token in text.split_whitespace() {
        let digest = Sha256::digest(token.as_bytes());
        let bucket = ((digest[0] as usize) << 8 | digest[1] as usize) % dim;
        let sign = if digest[2] % 2 == 0 { 1.0 } else { -1.0 };
        values[bucket] += sign;
    }

    let norm = values.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in &mut values {
            *value /= norm;
        }
    }
    values
}
