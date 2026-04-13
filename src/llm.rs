use async_trait::async_trait;
use anyhow::{Result, anyhow};
use crate::agent::{Message, AnthropicResponse, ContentBlock};
use serde_json::{json, Value};
use std::collections::HashMap;

#[async_trait]
pub trait LlmBackend: Send + Sync {
    async fn ask(&self, messages: &[Message], tools: &[Value]) -> Result<AnthropicResponse>;
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;
    async fn summarize(&self, messages: &[Message]) -> Result<String>;
}

pub struct MockLlm;
#[async_trait]
impl LlmBackend for MockLlm {
    async fn ask(&self, messages: &[Message], _tools: &[Value]) -> Result<AnthropicResponse> {
        let last_msg_json = &messages.last().unwrap().content;
        let last_msg_text = if last_msg_json.is_array() {
            last_msg_json.get(0).and_then(|b| b.get("text")).and_then(|v| v.as_str()).unwrap_or("")
        } else {
            last_msg_json.as_str().unwrap_or("")
        };
        Ok(AnthropicResponse {
            content: vec![ContentBlock::Text { text: format!("Mock Response: {}", last_msg_text) }],
            stop_reason: Some("end_turn".to_string()),
            usage: Some(crate::agent::TokenUsage { input_tokens: 10, output_tokens: 20 }),
        })
    }
    async fn embed(&self, _text: &str) -> Result<Vec<f32>> { Ok(vec![0.1; 128]) }
    async fn summarize(&self, _messages: &[Message]) -> Result<String> { Ok("Mock summary".into()) }
}

pub struct OpenAiCompatibleBackend {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
}

pub struct OllamaBackend {
    pub base_url: String,
    pub model: String,
    pub thinking: bool,
}

#[async_trait]
impl LlmBackend for OpenAiCompatibleBackend {
    async fn ask(&self, messages: &[Message], tools: &[Value]) -> Result<AnthropicResponse> {
        let client = reqwest::Client::new();
        let openai_messages = to_openai_messages(messages);
        let openai_tools: Vec<Value> = tools.iter().map(|t| json!({
            "type": "function", "function": { "name": t["name"], "description": t["description"], "parameters": t["input_schema"] }
        })).collect();

        let mut body = json!({ "model": self.model, "messages": openai_messages });
        if !openai_tools.is_empty() { body["tools"] = json!(openai_tools); }

        let mut request = client.post(&format!("{}/v1/chat/completions", self.base_url.trim_end_matches('/')));
        if !self.api_key.is_empty() {
            request = request.header("Authorization", format!("Bearer {}", self.api_key));
        }
        let res = request.json(&body).send().await?;

        if !res.status().is_success() { return Err(anyhow!("API Error: {}", res.text().await?)); }
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
                    name: tc["function"]["name"].as_str().unwrap_or_default().to_string(),
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
            stop_reason: resp_json["choices"][0]["finish_reason"].as_str().map(|s| s.to_string()),
            usage,
        })
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let client = reqwest::Client::new();
        let body = json!({ "model": "text-embedding-3-small", "input": text });
        let mut request = client.post(&format!("{}/v1/embeddings", self.base_url.trim_end_matches('/')));
        if !self.api_key.is_empty() {
            request = request.header("Authorization", format!("Bearer {}", self.api_key));
        }
        let res = request.json(&body).send().await?;
        let resp_json: Value = res.json().await?;
        let embedding = resp_json["data"][0]["embedding"].as_array()
            .ok_or_else(|| anyhow!("Failed to parse embedding"))?
            .iter().map(|v| v.as_f64().unwrap() as f32).collect();
        Ok(embedding)
    }

    async fn summarize(&self, messages: &[Message]) -> Result<String> {
        let client = reqwest::Client::new();
        let mut msgs = messages.to_vec();
        msgs.push(Message { role: "user".to_string(), content: json!("Summarize concisely.") });
        let body = json!({ "model": self.model, "messages": to_openai_messages(&msgs) });
        let mut request = client.post(&format!("{}/v1/chat/completions", self.base_url.trim_end_matches('/')));
        if !self.api_key.is_empty() {
            request = request.header("Authorization", format!("Bearer {}", self.api_key));
        }
        let res = request.json(&body).send().await?;
        let resp_json: Value = res.json().await?;
        Ok(extract_message_text(resp_json["choices"][0]["message"].get("content")).unwrap_or_default())
    }
}

fn to_openai_messages(messages: &[Message]) -> Vec<Value> {
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

fn render_openai_message_content(content: &Value) -> String {
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

fn extract_message_text(content: Option<&Value>) -> Option<String> {
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

fn parse_tool_arguments(arguments: &Value) -> Result<Value> {
    match arguments {
        Value::String(raw) => serde_json::from_str(raw).map_err(Into::into),
        Value::Object(_) => Ok(arguments.clone()),
        Value::Null => Ok(json!({})),
        _ => Err(anyhow!("tool arguments must be a string or object")),
    }
}

#[async_trait]
impl LlmBackend for OllamaBackend {
    async fn ask(&self, messages: &[Message], tools: &[Value]) -> Result<AnthropicResponse> {
        let client = reqwest::Client::new();
        let endpoint = format!("{}/api/chat", self.base_url.trim_end_matches('/'));
        let mut body = json!({
            "model": self.model,
            "messages": to_ollama_messages(messages),
            "stream": false,
            "think": self.thinking,
        });
        if !tools.is_empty() {
            body["tools"] = Value::Array(
                tools.iter()
                    .map(|tool| {
                        json!({
                            "type": "function",
                            "function": {
                                "name": tool["name"],
                                "description": tool["description"],
                                "parameters": tool["input_schema"],
                            }
                        })
                    })
                    .collect(),
            );
        }

        let res = client
            .post(&endpoint)
            .json(&body)
            .send()
            .await?;

        if !res.status().is_success() {
            return Err(anyhow!("API Error at {}: {}", endpoint, res.text().await?));
        }
        let resp_json: Value = res.json().await?;
        let message = &resp_json["message"];
        let mut content = Vec::new();
        if let Some(text) = message.get("content").and_then(Value::as_str) {
            if !text.trim().is_empty() {
                content.push(ContentBlock::Text {
                    text: text.to_string(),
                });
            }
        }
        if let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) {
            for (idx, call) in tool_calls.iter().enumerate() {
                content.push(ContentBlock::ToolUse {
                    id: format!("ollama-tool-{}", idx + 1),
                    name: call["function"]["name"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                    input: parse_tool_arguments(&call["function"]["arguments"])?,
                });
            }
        }

        Ok(AnthropicResponse {
            content,
            stop_reason: resp_json
                .get("done_reason")
                .and_then(Value::as_str)
                .map(str::to_string),
            usage: Some(crate::agent::TokenUsage {
                input_tokens: resp_json
                    .get("prompt_eval_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u32,
                output_tokens: resp_json
                    .get("eval_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u32,
            }),
        })
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        Ok(hashed_embedding(text, 256))
    }

    async fn summarize(&self, messages: &[Message]) -> Result<String> {
        let mut messages = messages.to_vec();
        messages.push(Message {
            role: "user".to_string(),
            content: json!("Summarize concisely."),
        });
        let response = self.ask(&messages, &[]).await?;
        let text = response
            .content
            .into_iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        Ok(text)
    }
}

fn to_ollama_messages(messages: &[Message]) -> Vec<Value> {
    let mut ollama_messages = Vec::new();
    let mut tool_names_by_id = HashMap::new();
    for message in messages {
        match message.role.as_str() {
            "system" => ollama_messages.push(json!({
                "role": "system",
                "content": render_openai_message_content(&message.content),
            })),
            "assistant" => ollama_messages.push(render_ollama_assistant_message(
                &message.content,
                &mut tool_names_by_id,
            )),
            "user" => {
                if let Some(tool_result) =
                    extract_ollama_tool_result_message(&message.content, &tool_names_by_id)
                {
                    ollama_messages.push(tool_result);
                } else {
                    ollama_messages.push(json!({
                        "role": "user",
                        "content": render_openai_message_content(&message.content),
                    }));
                }
            }
            other => ollama_messages.push(json!({
                "role": other,
                "content": render_openai_message_content(&message.content),
            })),
        }
    }
    ollama_messages
}

fn render_ollama_assistant_message(
    content: &Value,
    tool_names_by_id: &mut HashMap<String, String>,
) -> Value {
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
                    let tool_name = item
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    if let Some(id) = item.get("id").and_then(Value::as_str) {
                        tool_names_by_id.insert(id.to_string(), tool_name.clone());
                    }
                    tool_calls.push(json!({
                        "function": {
                            "name": tool_name,
                            "arguments": item.get("input").cloned().unwrap_or_else(|| json!({})),
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
        "content": text_parts.join("\n\n"),
    });
    if !tool_calls.is_empty() {
        message["tool_calls"] = Value::Array(tool_calls);
    }
    message
}

fn extract_ollama_tool_result_message(
    content: &Value,
    tool_names_by_id: &HashMap<String, String>,
) -> Option<Value> {
    let items = content.as_array()?;
    if items.len() != 1 {
        return None;
    }
    let item = &items[0];
    if item.get("type").and_then(Value::as_str) != Some("tool_result") {
        return None;
    }
    let tool_use_id = item.get("tool_use_id").and_then(Value::as_str).unwrap_or_default();
    Some(json!({
        "role": "tool",
        "tool_name": tool_names_by_id.get(tool_use_id).cloned().unwrap_or_default(),
        "content": item.get("content").and_then(Value::as_str).unwrap_or(""),
    }))
}

fn hashed_embedding(text: &str, dim: usize) -> Vec<f32> {
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

pub struct CodexBackend { pub api_key: String, pub base_url: String, pub model: String }
#[async_trait]
impl LlmBackend for CodexBackend {
    async fn ask(&self, m: &[Message], t: &[Value]) -> Result<AnthropicResponse> { 
        OpenAiCompatibleBackend { api_key: self.api_key.clone(), base_url: self.base_url.clone(), model: self.model.clone() }.ask(m, t).await 
    }
    async fn embed(&self, t: &str) -> Result<Vec<f32>> { 
        OpenAiCompatibleBackend { api_key: self.api_key.clone(), base_url: self.base_url.clone(), model: self.model.clone() }.embed(t).await 
    }
    async fn summarize(&self, m: &[Message]) -> Result<String> { 
        OpenAiCompatibleBackend { api_key: self.api_key.clone(), base_url: self.base_url.clone(), model: self.model.clone() }.summarize(m).await 
    }
}

pub struct GeminiBackend { pub api_key: String, pub model: String }
#[async_trait]
impl LlmBackend for GeminiBackend {
    async fn ask(&self, _: &[Message], _: &[Value]) -> Result<AnthropicResponse> { Err(anyhow!("Gemini pending")) }
    async fn embed(&self, _: &str) -> Result<Vec<f32>> { Err(anyhow!("Gemini pending")) }
    async fn summarize(&self, _: &[Message]) -> Result<String> { Err(anyhow!("Gemini pending")) }
}

#[cfg(test)]
mod tests {
    use super::{
        extract_message_text, parse_tool_arguments, to_ollama_messages, to_openai_messages,
        Message,
    };
    use serde_json::json;

    #[test]
    fn converts_assistant_tool_history_to_openai_messages() {
        let messages = vec![
            Message {
                role: "assistant".to_string(),
                content: json!([
                    {"type":"text","text":"Need a tool."},
                    {"type":"tool_use","id":"tool-1","name":"read_file","input":{"path":"Cargo.toml"}}
                ]),
            },
            Message {
                role: "user".to_string(),
                content: json!([
                    {"type":"tool_result","tool_use_id":"tool-1","content":"[package]"}
                ]),
            },
        ];

        let openai_messages = to_openai_messages(&messages);
        assert_eq!(openai_messages[0]["role"], "assistant");
        assert_eq!(openai_messages[0]["tool_calls"][0]["function"]["name"], "read_file");
        assert_eq!(openai_messages[1]["role"], "tool");
        assert_eq!(openai_messages[1]["tool_call_id"], "tool-1");
    }

    #[test]
    fn parses_tool_arguments_from_string_and_object() {
        assert_eq!(
            parse_tool_arguments(&json!("{\"path\":\"Cargo.toml\"}")).unwrap(),
            json!({"path":"Cargo.toml"})
        );
        assert_eq!(
            parse_tool_arguments(&json!({"path":"Cargo.toml"})).unwrap(),
            json!({"path":"Cargo.toml"})
        );
    }

    #[test]
    fn extracts_text_from_openai_content_array() {
        assert_eq!(
            extract_message_text(Some(&json!([
                {"type":"text","text":"hello"},
                {"type":"text","text":"world"}
            ]))),
            Some("hello\n\nworld".to_string())
        );
    }

    #[test]
    fn converts_tool_history_to_ollama_messages() {
        let messages = vec![
            Message {
                role: "assistant".to_string(),
                content: json!([
                    {"type":"text","text":"Need a tool."},
                    {"type":"tool_use","id":"tool-1","name":"read_file","input":{"path":"Cargo.toml"}}
                ]),
            },
            Message {
                role: "user".to_string(),
                content: json!([
                    {"type":"tool_result","tool_use_id":"tool-1","content":"[package]"}
                ]),
            },
        ];

        let ollama_messages = to_ollama_messages(&messages);
        assert_eq!(ollama_messages[0]["role"], "assistant");
        assert_eq!(ollama_messages[0]["tool_calls"][0]["function"]["name"], "read_file");
        assert_eq!(ollama_messages[1]["role"], "tool");
        assert_eq!(ollama_messages[1]["tool_name"], "read_file");
    }

}
