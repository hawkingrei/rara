use async_trait::async_trait;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::agent::{AnthropicResponse, ContentBlock, Message};

use super::shared::{
    extract_message_text, http_client_for_target, model_context_budget, parse_tool_arguments,
    render_openai_message_content, ContextBudget, LlmBackend,
};

pub struct OpenAiCompatibleBackend {
    pub client: reqwest::Client,
    pub api_key: String,
    pub base_url: String,
    pub model: String,
}

impl OpenAiCompatibleBackend {
    pub fn new(api_key: String, base_url: String, model: String) -> Result<Self> {
        Ok(Self {
            client: http_client_for_target(&base_url)?,
            api_key,
            base_url,
            model,
        })
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

        let mut request = self.client.post(&format!(
            "{}/v1/chat/completions",
            self.base_url.trim_end_matches('/')
        ));
        if !self.api_key.is_empty() {
            request = request.header("Authorization", format!("Bearer {}", self.api_key));
        }
        let res = request.json(&body).send().await?;

        if !res.status().is_success() {
            return Err(anyhow!("API Error: {}", res.text().await?));
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
        let mut request = self.client.post(&format!(
            "{}/v1/embeddings",
            self.base_url.trim_end_matches('/')
        ));
        if !self.api_key.is_empty() {
            request = request.header("Authorization", format!("Bearer {}", self.api_key));
        }
        let res = request.json(&body).send().await?;
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
        let mut request = self.client.post(&format!(
            "{}/v1/chat/completions",
            self.base_url.trim_end_matches('/')
        ));
        if !self.api_key.is_empty() {
            request = request.header("Authorization", format!("Bearer {}", self.api_key));
        }
        let res = request.json(&body).send().await?;
        let resp_json: Value = res.json().await?;
        Ok(extract_message_text(resp_json["choices"][0]["message"].get("content"))
            .unwrap_or_default())
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
    let mut text_parts = Vec::new();
    let mut tool_calls = Vec::new();
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
                    tool_calls.push(json!({
                        "id": item.get("id").and_then(Value::as_str).unwrap_or_default(),
                        "type": "function",
                        "function": {
                            "name": item.get("name").and_then(Value::as_str).unwrap_or_default(),
                            "arguments": serde_json::to_string(
                                &item.get("input").cloned().unwrap_or_else(|| json!({}))
                            ).unwrap_or_else(|_| "{}".to_string()),
                        }
                    }));
                }
                _ => {}
            }
        }
    } else if let Some(text) = content.as_str() {
        text_parts.push(text.to_string());
    }

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
    let items = content.as_array()?;
    if items.len() != 1 {
        return None;
    }
    let item = &items[0];
    if item.get("type").and_then(Value::as_str) != Some("tool_result") {
        return None;
    }
    Some(json!({
        "role": "tool",
        "tool_call_id": item.get("tool_use_id").and_then(Value::as_str).unwrap_or_default(),
        "content": item.get("content").and_then(Value::as_str).unwrap_or(""),
    }))
}

pub struct CodexBackend {
    inner: OpenAiCompatibleBackend,
}

impl CodexBackend {
    pub fn new(api_key: String, base_url: String, model: String) -> Result<Self> {
        Ok(Self {
            inner: OpenAiCompatibleBackend::new(api_key, base_url, model)?,
        })
    }
}

#[async_trait]
impl LlmBackend for CodexBackend {
    async fn ask(&self, m: &[Message], t: &[Value]) -> Result<AnthropicResponse> {
        self.inner.ask(m, t).await
    }

    async fn embed(&self, t: &str) -> Result<Vec<f32>> {
        self.inner.embed(t).await
    }

    async fn summarize(&self, m: &[Message], instruction: &str) -> Result<String> {
        self.inner.summarize(m, instruction).await
    }

    fn context_budget(&self, messages: &[Message], tools: &[Value]) -> Option<ContextBudget> {
        self.inner.context_budget(messages, tools)
    }
}

pub struct GeminiBackend {
    pub api_key: String,
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
