mod codex;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{json, Value};

use crate::agent::Message;
use crate::config::OpenAiEndpointKind;
use crate::llm::{ContentBlock, LlmResponse, TokenUsage};
use crate::redaction::{redact_secrets, sanitize_url_for_display};

use super::shared::{
    collect_assistant_content, extract_message_text, http_client_for_target, model_context_budget,
    parse_tool_arguments, render_openai_message_content, ContextBudget, LlmBackend,
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
    pub endpoint_kind: OpenAiEndpointKind,
}

impl OpenAiCompatibleBackend {
    pub fn new(api_key: Option<SecretString>, base_url: String, model: String) -> Result<Self> {
        Self::new_with_endpoint_kind(api_key, base_url, model, OpenAiEndpointKind::Custom)
    }

    pub fn new_with_endpoint_kind(
        api_key: Option<SecretString>,
        base_url: String,
        model: String,
        endpoint_kind: OpenAiEndpointKind,
    ) -> Result<Self> {
        Ok(Self {
            client: http_client_for_target(&base_url)?,
            api_key,
            base_url,
            model,
            endpoint_kind,
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
        let openai_messages = to_openai_messages_for_endpoint(messages, self.endpoint_kind);
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
        parse_chat_completion_response(&resp_json, self.endpoint_kind)
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
        let body = json!({
            "model": self.model,
            "messages": to_openai_messages_for_endpoint(&msgs, self.endpoint_kind),
        });
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
    to_openai_messages_for_endpoint(messages, OpenAiEndpointKind::Custom)
}

pub(super) fn to_openai_messages_for_endpoint(
    messages: &[Message],
    endpoint_kind: OpenAiEndpointKind,
) -> Vec<Value> {
    let mut openai_messages = Vec::new();
    let mut pending_tool_call_ids = Vec::new();
    for message in messages {
        match message.role.as_str() {
            "system" => {
                flush_missing_tool_results(&mut openai_messages, &mut pending_tool_call_ids);
                openai_messages.push(json!({
                    "role": "system",
                    "content": render_openai_message_content(&message.content),
                }));
            }
            "assistant" => {
                flush_missing_tool_results(&mut openai_messages, &mut pending_tool_call_ids);
                let assistant_message =
                    render_openai_assistant_message(&message.content, endpoint_kind);
                pending_tool_call_ids = assistant_tool_call_ids(&assistant_message);
                openai_messages.push(assistant_message);
            }
            "user" => {
                let (tool_results, user_content) = split_tool_result_blocks(&message.content);
                for (tool_use_id, tool_content) in tool_results {
                    if remove_pending_tool_call(&mut pending_tool_call_ids, &tool_use_id) {
                        openai_messages.push(render_openai_tool_result_message(
                            &tool_use_id,
                            &tool_content,
                        ));
                    }
                }
                if let Some(user_content) = user_content {
                    flush_missing_tool_results(&mut openai_messages, &mut pending_tool_call_ids);
                    openai_messages.push(json!({
                        "role": "user",
                        "content": render_openai_message_content(&user_content),
                    }));
                }
            }
            other => {
                flush_missing_tool_results(&mut openai_messages, &mut pending_tool_call_ids);
                openai_messages.push(json!({
                    "role": other,
                    "content": render_openai_message_content(&message.content),
                }));
            }
        }
    }
    flush_missing_tool_results(&mut openai_messages, &mut pending_tool_call_ids);
    openai_messages
}

fn render_openai_assistant_message(content: &Value, endpoint_kind: OpenAiEndpointKind) -> Value {
    let (text_parts, assistant_tool_uses) = collect_assistant_content(content);
    let tool_calls = assistant_tool_uses
        .into_iter()
        .filter(|tool_use| !tool_use.id.trim().is_empty() && !tool_use.name.trim().is_empty())
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
    if endpoint_kind == OpenAiEndpointKind::Deepseek {
        if let Some(reasoning_content) =
            provider_metadata_string(content, "deepseek", "reasoning_content")
        {
            message["reasoning_content"] = Value::String(reasoning_content.to_string());
        }
    }
    message
}

fn provider_metadata_string<'a>(content: &'a Value, provider: &str, key: &str) -> Option<&'a str> {
    content.as_array()?.iter().find_map(|item| {
        if item.get("type").and_then(Value::as_str) != Some("provider_metadata") {
            return None;
        }
        if item.get("provider").and_then(Value::as_str) != Some(provider) {
            return None;
        }
        if item.get("key").and_then(Value::as_str) != Some(key) {
            return None;
        }
        item.get("value")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    })
}

fn assistant_tool_call_ids(message: &Value) -> Vec<String> {
    message
        .get("tool_calls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|tool_call| tool_call.get("id").and_then(Value::as_str))
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .collect()
}

fn remove_pending_tool_call(pending_tool_call_ids: &mut Vec<String>, tool_use_id: &str) -> bool {
    let Some(pos) = pending_tool_call_ids
        .iter()
        .position(|id| id == tool_use_id)
    else {
        return false;
    };
    pending_tool_call_ids.remove(pos);
    true
}

fn flush_missing_tool_results(
    openai_messages: &mut Vec<Value>,
    pending_tool_call_ids: &mut Vec<String>,
) {
    for tool_use_id in pending_tool_call_ids.drain(..) {
        openai_messages.push(render_openai_tool_result_message(
            &tool_use_id,
            "Tool execution was interrupted before a result was recorded.",
        ));
    }
}

fn split_tool_result_blocks(content: &Value) -> (Vec<(String, String)>, Option<Value>) {
    let Some(items) = content.as_array() else {
        return (Vec::new(), Some(content.clone()));
    };

    let mut tool_results = Vec::new();
    let mut user_blocks = Vec::new();
    for item in items {
        if item.get("type").and_then(Value::as_str) == Some("tool_result") {
            let Some(tool_use_id) = item.get("tool_use_id").and_then(Value::as_str) else {
                continue;
            };
            tool_results.push((
                tool_use_id.to_string(),
                item.get("content")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            ));
        } else {
            user_blocks.push(item.clone());
        }
    }

    let user_content = (!user_blocks.is_empty()).then_some(Value::Array(user_blocks));
    (tool_results, user_content)
}

pub(super) fn parse_chat_completion_response(
    resp_json: &Value,
    endpoint_kind: OpenAiEndpointKind,
) -> Result<LlmResponse> {
    let first_choice = resp_json
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .ok_or_else(|| anyhow!("OpenAI-compatible response missing choices[0]"))?;
    let choice = first_choice
        .get("message")
        .ok_or_else(|| anyhow!("OpenAI-compatible response missing choices[0].message"))?;
    let mut content = Vec::new();
    if let Some(text) = extract_message_text(choice.get("content")) {
        if !text.trim().is_empty() {
            content.push(ContentBlock::Text { text });
        }
    }
    if endpoint_kind == OpenAiEndpointKind::Deepseek {
        if let Some(reasoning_content) = choice
            .get("reasoning_content")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            content.push(ContentBlock::ProviderMetadata {
                provider: "deepseek".to_string(),
                key: "reasoning_content".to_string(),
                value: Value::String(reasoning_content.to_string()),
            });
        }
    }
    if let Some(tool_calls) = choice["tool_calls"].as_array() {
        for (idx, tc) in tool_calls.iter().enumerate() {
            let id = tc
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.trim().is_empty())
                .ok_or_else(|| {
                    anyhow!("OpenAI-compatible response tool_calls[{idx}] missing id")
                })?;
            let function = tc.get("function").ok_or_else(|| {
                anyhow!("OpenAI-compatible response tool_calls[{idx}] missing function")
            })?;
            let name = function
                .get("name")
                .and_then(Value::as_str)
                .filter(|name| !name.trim().is_empty())
                .ok_or_else(|| {
                    anyhow!("OpenAI-compatible response tool_calls[{idx}].function missing name")
                })?;
            let arguments = function.get("arguments").ok_or_else(|| {
                anyhow!("OpenAI-compatible response tool_calls[{idx}].function missing arguments")
            })?;
            content.push(ContentBlock::ToolUse {
                id: id.to_string(),
                name: name.to_string(),
                input: parse_tool_arguments(arguments)?,
            });
        }
    }
    let usage = resp_json.get("usage").map(|u| TokenUsage {
        input_tokens: u["prompt_tokens"].as_u64().unwrap_or(0) as u32,
        output_tokens: u["completion_tokens"].as_u64().unwrap_or(0) as u32,
    });
    Ok(LlmResponse {
        content,
        stop_reason: first_choice
            .get("finish_reason")
            .and_then(Value::as_str)
            .map(str::to_string),
        usage,
    })
}

fn render_openai_tool_result_message(tool_use_id: &str, tool_content: &str) -> Value {
    json!({
        "role": "tool",
        "tool_call_id": tool_use_id,
        "content": tool_content,
    })
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
