use async_trait::async_trait;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::time::Duration;
use url::Url;

use crate::agent::{AnthropicResponse, ContentBlock, Message};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextBudget {
    pub context_window_tokens: usize,
    pub reserved_output_tokens: usize,
    pub compact_threshold_tokens: usize,
}

#[async_trait]
pub trait LlmBackend: Send + Sync {
    async fn ask(&self, messages: &[Message], tools: &[Value]) -> Result<AnthropicResponse>;
    async fn ask_streaming(
        &self,
        messages: &[Message],
        tools: &[Value],
        _on_text_delta: &mut (dyn FnMut(String) + Send),
    ) -> Result<AnthropicResponse> {
        self.ask(messages, tools).await
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
    async fn ask(&self, messages: &[Message], _tools: &[Value]) -> Result<AnthropicResponse> {
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
        Ok(AnthropicResponse {
            content: vec![ContentBlock::Text {
                text: format!("Mock Response: {}", last_msg_text),
            }],
            stop_reason: Some("end_turn".to_string()),
            usage: Some(crate::agent::TokenUsage {
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

pub(super) fn http_client_for_target(base_url: &str) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(HTTP_CONNECT_TIMEOUT)
        .read_timeout(HTTP_READ_TIMEOUT);
    if should_bypass_proxy(base_url) {
        builder = builder.no_proxy();
    }
    builder.build().map_err(Into::into)
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
    let context_window_tokens = if canonical.contains("gpt-5")
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
