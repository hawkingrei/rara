mod codex;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{json, Value};

use crate::agent::Message;
use crate::llm::{ContentBlock, LlmResponse, TokenUsage};
use crate::redaction::{redact_secrets, sanitize_url_for_display};

use super::shared::{
    collect_assistant_content, extract_message_text, extract_single_tool_result,
    http_client_for_target, model_context_budget, parse_tool_arguments,
    render_openai_message_content, ContextBudget, LlmBackend,
};

#[cfg(test)]
pub(crate) use self::codex::{
    apply_codex_stream_event, build_codex_responses_request, build_codex_stream_response,
    parse_codex_response, to_codex_input_items,
};

pub struct OpenAiCompatibleBackend {
    pub client: reqwest::Client,
    pub api_key: Option<SecretString>,
    pub base_url: String,
    pub model: String,
}

impl OpenAiCompatibleBackend {
    pub fn new(api_key: Option<SecretString>, base_url: String, model: String) -> Result<Self> {
        Ok(Self {
            client: http_client_for_target(&base_url)?,
            api_key,
            base_url,
            model,
        })
    }

    fn endpoint_url(&self, path: &str) -> String {
        let base = self.base_url.trim_end_matches('/');
        let path = path.trim_start_matches('/');
        let normalized_base = if base.ends_with("/v1") {
            base.to_string()
        } else {
            format!("{base}/v1")
        };
        format!("{normalized_base}/{path}")
    }
}

#[async_trait]
impl LlmBackend for OpenAiCompatibleBackend {
    async fn ask(&self, messages: &[Message], tools: &[Value]) -> Result<LlmResponse> {
        let openai_messages = to_openai_messages(messages);
        let openai_tools: Vec<Value> = tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t["name"],
                        "description": t["description"],
                        "parameters": t["input_schema"]
                    }
                })
            })
            .collect();

        let mut body = json!({ "model": self.model, "messages": openai_messages });
        if !openai_tools.is_empty() {
            body["tools"] = json!(openai_tools);
        }

        let completions_url = self.endpoint_url("chat/completions");
        let mut request = self.client.post(&completions_url);
        if let Some(api_key) = self.api_key.as_ref().map(SecretString::expose_secret) {
            if !api_key.is_empty() {
                request = request.header("Authorization", format!("Bearer {api_key}"));
            }
        }
        let res = request.json(&body).send().await?;

        if !res.status().is_success() {
            return Err(anyhow!(
                "API Error at {}: {}",
                sanitize_url_for_display(&completions_url),
                redact_secrets(res.text().await?)
            ));
        }
        let resp_json: Value = res.json().await?;
        let choice = &resp_json["choices"][0]["message"];
        let mut content = Vec::new();
        if let Some(text) = extract_message_text(choice.get("content")) {
            if !text.trim().is_empty() {
                content.push(ContentBlock::Text { text });
            }
        }
        if let Some(tool_calls) = choice["tool_calls"].as_array() {
            for tc in tool_calls {
                content.push(ContentBlock::ToolUse {
                    id: tc["id"].as_str().unwrap_or_default().to_string(),
                    name: tc["function"]["name"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                    input: parse_tool_arguments(&tc["function"]["arguments"])?,
                });
            }
        }
        let usage = resp_json.get("usage").map(|u| TokenUsage {
            input_tokens: u["prompt_tokens"].as_u64().unwrap_or(0) as u32,
            output_tokens: u["completion_tokens"].as_u64().unwrap_or(0) as u32,
        });
        Ok(LlmResponse {
            content,
            stop_reason: resp_json["choices"][0]["finish_reason"]
                .as_str()
                .map(|s| s.to_string()),
            usage,
        })
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let body = json!({ "model": "text-embedding-3-small", "input": text });
        let embeddings_url = self.endpoint_url("embeddings");
        let mut request = self.client.post(&embeddings_url);
        if let Some(api_key) = self.api_key.as_ref().map(SecretString::expose_secret) {
            if !api_key.is_empty() {
                request = request.header("Authorization", format!("Bearer {api_key}"));
            }
        }
        let res = request.json(&body).send().await?;
        if !res.status().is_success() {
            return Err(anyhow!(
                "API Error at {}: {}",
                sanitize_url_for_display(&embeddings_url),
                redact_secrets(res.text().await?)
            ));
        }
        let resp_json: Value = res.json().await?;
        let embedding = resp_json["data"][0]["embedding"]
            .as_array()
            .ok_or_else(|| anyhow!("Failed to parse embedding"))?
            .iter()
            .map(|v| v.as_f64().unwrap() as f32)
            .collect();
        Ok(embedding)
    }

    async fn summarize(&self, messages: &[Message], instruction: &str) -> Result<String> {
        let mut msgs = messages.to_vec();
        msgs.push(Message {
            role: "user".to_string(),
            content: json!(instruction),
        });
        let body = json!({ "model": self.model, "messages": to_openai_messages(&msgs) });
        let completions_url = self.endpoint_url("chat/completions");
        let mut request = self.client.post(&completions_url);
        if let Some(api_key) = self.api_key.as_ref().map(SecretString::expose_secret) {
            if !api_key.is_empty() {
                request = request.header("Authorization", format!("Bearer {api_key}"));
            }
        }
        let res = request.json(&body).send().await?;
        if !res.status().is_success() {
            return Err(anyhow!(
                "API Error at {}: {}",
                sanitize_url_for_display(&completions_url),
                redact_secrets(res.text().await?)
            ));
        }
        let resp_json: Value = res.json().await?;
        Ok(
            extract_message_text(resp_json["choices"][0]["message"].get("content"))
                .unwrap_or_default(),
        )
    }

    fn context_budget(&self, _messages: &[Message], _tools: &[Value]) -> Option<ContextBudget> {
        model_context_budget(self.model.as_str())
    }
}

pub(super) fn to_openai_messages(messages: &[Message]) -> Vec<Value> {
    let mut openai_messages = Vec::new();
    for message in messages {
        match message.role.as_str() {
            "system" => openai_messages.push(json!({
                "role": "system",
                "content": render_openai_message_content(&message.content),
            })),
            "assistant" => openai_messages.push(render_openai_assistant_message(&message.content)),
            "user" => {
                if let Some(tool_result) = extract_tool_result_message(&message.content) {
                    openai_messages.push(tool_result);
                } else {
                    openai_messages.push(json!({
                        "role": "user",
                        "content": render_openai_message_content(&message.content),
                    }));
                }
            }
            other => openai_messages.push(json!({
                "role": other,
                "content": render_openai_message_content(&message.content),
            })),
        }
    }
    openai_messages
}

fn render_openai_assistant_message(content: &Value) -> Value {
    let (text_parts, assistant_tool_uses) = collect_assistant_content(content);
    let tool_calls = assistant_tool_uses
        .into_iter()
        .map(|tool_use| {
            json!({
                "id": tool_use.id,
                "type": "function",
                "function": {
                    "name": tool_use.name,
                    "arguments": serde_json::to_string(&tool_use.input)
                        .unwrap_or_else(|_| "{}".to_string()),
                }
            })
        })
        .collect::<Vec<_>>();

    let mut message = json!({
        "role": "assistant",
        "content": if text_parts.is_empty() {
            Value::Null
        } else {
            Value::String(text_parts.join("\n\n"))
        },
    });
    if !tool_calls.is_empty() {
        message["tool_calls"] = Value::Array(tool_calls);
    }
    message
}

fn extract_tool_result_message(content: &Value) -> Option<Value> {
    let (tool_use_id, tool_content) = extract_single_tool_result(content)?;
    Some(json!({
        "role": "tool",
        "tool_call_id": tool_use_id,
        "content": tool_content,
    }))
}

pub struct CodexBackend {
    reasoning_effort: Option<String>,
    client: reqwest::Client,
    api_key: Option<SecretString>,
    base_url: String,
    model: String,
}

pub struct GeminiBackend {
    pub api_key: SecretString,
    pub model: String,
}

#[async_trait]
impl LlmBackend for GeminiBackend {
    async fn ask(&self, _: &[Message], _: &[Value]) -> Result<LlmResponse> {
        Err(anyhow!("Gemini pending"))
    }

    async fn embed(&self, _: &str) -> Result<Vec<f32>> {
        Err(anyhow!("Gemini pending"))
    }

    async fn summarize(&self, _: &[Message], _: &str) -> Result<String> {
        Err(anyhow!("Gemini pending"))
    }

    fn context_budget(&self, _messages: &[Message], _tools: &[Value]) -> Option<ContextBudget> {
        model_context_budget(self.model.as_str())
    }
}
