use async_trait::async_trait;
use anyhow::{Result, anyhow};
use crate::agent::{Message, AnthropicResponse, ContentBlock};
use serde_json::{json, Value};

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

#[async_trait]
impl LlmBackend for OpenAiCompatibleBackend {
    async fn ask(&self, messages: &[Message], tools: &[Value]) -> Result<AnthropicResponse> {
        let client = reqwest::Client::new();
        let openai_messages: Vec<Value> = messages.iter().map(|m| json!({ "role": m.role, "content": m.content })).collect();
        let openai_tools: Vec<Value> = tools.iter().map(|t| json!({
            "type": "function", "function": { "name": t["name"], "description": t["description"], "parameters": t["input_schema"] }
        })).collect();

        let mut body = json!({ "model": self.model, "messages": openai_messages });
        if !openai_tools.is_empty() { body["tools"] = json!(openai_tools); }

        let res = client.post(&format!("{}/v1/chat/completions", self.base_url.trim_end_matches('/')))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body).send().await?;

        if !res.status().is_success() { return Err(anyhow!("API Error: {}", res.text().await?)); }
        let resp_json: Value = res.json().await?;
        let choice = &resp_json["choices"][0]["message"];
        let mut content = Vec::new();
        if let Some(text) = choice["content"].as_str() { content.push(ContentBlock::Text { text: text.to_string() }); }
        if let Some(tool_calls) = choice["tool_calls"].as_array() {
            for tc in tool_calls {
                content.push(ContentBlock::ToolUse {
                    id: tc["id"].as_str().unwrap_or_default().to_string(),
                    name: tc["function"]["name"].as_str().unwrap_or_default().to_string(),
                    input: serde_json::from_str(tc["function"]["arguments"].as_str().unwrap_or("{}"))?,
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
        let res = client.post(&format!("{}/v1/embeddings", self.base_url.trim_end_matches('/')))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body).send().await?;
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
        let body = json!({ "model": self.model, "messages": msgs });
        let res = client.post(&format!("{}/v1/chat/completions", self.base_url.trim_end_matches('/')))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body).send().await?;
        let resp_json: Value = res.json().await?;
        Ok(resp_json["choices"][0]["message"]["content"].as_str().unwrap_or("").to_string())
    }
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
