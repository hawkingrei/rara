use anyhow::{anyhow, Result};
use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{json, Value};

use crate::agent::{AnthropicResponse, ContentBlock, Message};
use crate::redaction::{redact_secrets, sanitize_url_for_display};

use super::shared::{
    collect_assistant_content, extract_message_text, extract_single_tool_result,
    http_client_for_target, model_context_budget, parse_tool_arguments,
    render_openai_message_content, ContextBudget, LlmBackend,
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
    async fn ask(&self, messages: &[Message], tools: &[Value]) -> Result<AnthropicResponse> {
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
        let usage = resp_json.get("usage").map(|u| crate::agent::TokenUsage {
            input_tokens: u["prompt_tokens"].as_u64().unwrap_or(0) as u32,
            output_tokens: u["completion_tokens"].as_u64().unwrap_or(0) as u32,
        });
        Ok(AnthropicResponse {
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
    inner: OpenAiCompatibleBackend,
}

impl CodexBackend {
    pub fn new(api_key: Option<SecretString>, base_url: String, model: String) -> Result<Self> {
        Ok(Self {
            inner: OpenAiCompatibleBackend::new(api_key, base_url, model)?,
        })
    }
}

#[async_trait]
impl LlmBackend for CodexBackend {
    async fn ask(&self, m: &[Message], t: &[Value]) -> Result<AnthropicResponse> {
        let body = build_codex_responses_request(&self.inner.model, m, t);
        let responses_url = self.inner.endpoint_url("responses");
        let mut request = self.inner.client.post(&responses_url);
        if let Some(api_key) = self.inner.api_key.as_ref().map(SecretString::expose_secret) {
            if !api_key.is_empty() {
                request = request.header("Authorization", format!("Bearer {api_key}"));
            }
        }
        let res = request.json(&body).send().await?;
        if !res.status().is_success() {
            return Err(anyhow!(
                "API Error at {}: {}",
                sanitize_url_for_display(&responses_url),
                redact_secrets(res.text().await?)
            ));
        }
        let resp_json: Value = res.json().await?;
        parse_codex_response(&resp_json)
    }

    async fn embed(&self, t: &str) -> Result<Vec<f32>> {
        self.inner.embed(t).await
    }

    async fn summarize(&self, m: &[Message], instruction: &str) -> Result<String> {
        let mut messages = m.to_vec();
        messages.push(Message {
            role: "user".to_string(),
            content: json!(instruction),
        });
        let response = self.ask(&messages, &[]).await?;
        Ok(response
            .content
            .into_iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text),
                ContentBlock::ToolUse { .. } => None,
            })
            .collect::<Vec<_>>()
            .join("\n\n"))
    }

    fn context_budget(&self, messages: &[Message], tools: &[Value]) -> Option<ContextBudget> {
        self.inner.context_budget(messages, tools)
    }
}

fn build_codex_responses_request(model: &str, messages: &[Message], tools: &[Value]) -> Value {
    let codex_tools = tools
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "name": tool["name"],
                "description": tool["description"],
                "parameters": tool["input_schema"],
            })
        })
        .collect::<Vec<_>>();

    json!({
        "model": model,
        "input": to_codex_input_items(messages),
        "tools": codex_tools,
        "tool_choice": "auto",
        "parallel_tool_calls": false,
        "store": false,
        "stream": false,
        "include": [],
    })
}

pub(super) fn to_codex_input_items(messages: &[Message]) -> Vec<Value> {
    let mut items = Vec::new();
    for message in messages {
        match message.role.as_str() {
            "assistant" => items.extend(render_codex_assistant_items(&message.content)),
            "user" => {
                if let Some(tool_result) = extract_codex_tool_result_item(&message.content) {
                    items.push(tool_result);
                } else {
                    items.push(render_codex_message(
                        message.role.as_str(),
                        &message.content,
                        false,
                    ));
                }
            }
            role => items.push(render_codex_message(role, &message.content, false)),
        }
    }
    items
}

fn render_codex_message(role: &str, content: &Value, assistant_output: bool) -> Value {
    let text = render_openai_message_content(content);
    let content_item_type = if assistant_output {
        "output_text"
    } else {
        "input_text"
    };
    json!({
        "type": "message",
        "role": role,
        "content": [{
            "type": content_item_type,
            "text": text,
        }],
    })
}

fn render_codex_assistant_items(content: &Value) -> Vec<Value> {
    let (text_parts, assistant_tool_uses) = collect_assistant_content(content);
    let mut items = Vec::new();
    if !text_parts.is_empty() {
        items.push(render_codex_message(
            "assistant",
            &Value::String(text_parts.join("\n\n")),
            true,
        ));
    }
    for tool_use in assistant_tool_uses {
        items.push(json!({
            "type": "function_call",
            "name": tool_use.name,
            "arguments": serde_json::to_string(&tool_use.input)
                .unwrap_or_else(|_| "{}".to_string()),
            "call_id": tool_use.id,
        }));
    }
    items
}

fn extract_codex_tool_result_item(content: &Value) -> Option<Value> {
    let (tool_use_id, tool_content) = extract_single_tool_result(content)?;
    Some(json!({
        "type": "function_call_output",
        "call_id": tool_use_id,
        "output": tool_content,
    }))
}

pub(super) fn parse_codex_response(resp_json: &Value) -> Result<AnthropicResponse> {
    let mut content = Vec::new();
    if let Some(items) = resp_json.get("output").and_then(Value::as_array) {
        for item in items {
            match item.get("type").and_then(Value::as_str) {
                Some("message") => {
                    if let Some(text) = extract_codex_output_text(item.get("content")) {
                        if !text.trim().is_empty() {
                            content.push(ContentBlock::Text { text });
                        }
                    }
                }
                Some("function_call") => {
                    let id = item
                        .get("call_id")
                        .or_else(|| item.get("id"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let name = item
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let input =
                        parse_tool_arguments(item.get("arguments").unwrap_or(&Value::Null))?;
                    content.push(ContentBlock::ToolUse { id, name, input });
                }
                _ => {}
            }
        }
    }
    let usage = resp_json.get("usage").map(|u| crate::agent::TokenUsage {
        input_tokens: u["input_tokens"].as_u64().unwrap_or(0) as u32,
        output_tokens: u["output_tokens"].as_u64().unwrap_or(0) as u32,
    });
    Ok(AnthropicResponse {
        content,
        stop_reason: resp_json
            .get("status")
            .and_then(Value::as_str)
            .map(|s| s.to_string()),
        usage,
    })
}

fn extract_codex_output_text(content: Option<&Value>) -> Option<String> {
    let items = content?.as_array()?;
    let texts = items
        .iter()
        .filter_map(|item| match item.get("type").and_then(Value::as_str) {
            Some("output_text") => item.get("text").and_then(Value::as_str).map(str::to_string),
            _ => None,
        })
        .collect::<Vec<_>>();
    if texts.is_empty() {
        None
    } else {
        Some(texts.join("\n\n"))
    }
}

pub struct GeminiBackend {
    pub api_key: SecretString,
    pub model: String,
}

#[async_trait]
impl LlmBackend for GeminiBackend {
    async fn ask(&self, _: &[Message], _: &[Value]) -> Result<AnthropicResponse> {
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
