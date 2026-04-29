mod codex;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use eventsource_stream::Eventsource;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{json, Value};

use crate::agent::Message;
use crate::config::OpenAiEndpointKind;
use crate::llm::{ContentBlock, LlmResponse, TokenUsage};
use crate::redaction::{redact_secrets, sanitize_url_for_display};

use super::shared::{
    collect_assistant_content, extract_message_text, http_client_for_target, model_context_budget,
    next_stream_item_with_idle_timeout, parse_tool_arguments, render_openai_message_content,
    ContextBudget, LlmBackend, LlmTurnMetadata,
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
    pub reasoning_effort: Option<String>,
    pub thinking: Option<bool>,
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
        Self::new_with_endpoint_kind_and_reasoning(
            api_key,
            base_url,
            model,
            endpoint_kind,
            None,
            None,
        )
    }

    pub fn new_with_endpoint_kind_and_reasoning(
        api_key: Option<SecretString>,
        base_url: String,
        model: String,
        endpoint_kind: OpenAiEndpointKind,
        reasoning_effort: Option<String>,
        thinking: Option<bool>,
    ) -> Result<Self> {
        Ok(Self {
            client: http_client_for_target(&base_url)?,
            api_key,
            base_url,
            model,
            endpoint_kind,
            reasoning_effort,
            thinking,
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
        self.ask_with_context(messages, tools, LlmTurnMetadata::default())
            .await
    }

    async fn ask_with_context(
        &self,
        messages: &[Message],
        tools: &[Value],
        metadata: LlmTurnMetadata,
    ) -> Result<LlmResponse> {
        let body = build_chat_completion_request_body(
            &self.model,
            messages,
            tools,
            self.endpoint_kind,
            self.reasoning_effort.as_deref(),
            self.thinking,
            metadata.clone(),
        );

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

    async fn ask_streaming_with_context(
        &self,
        messages: &[Message],
        tools: &[Value],
        metadata: LlmTurnMetadata,
        on_text_delta: &mut (dyn FnMut(String) + Send),
    ) -> Result<LlmResponse> {
        let mut body = build_chat_completion_request_body(
            &self.model,
            messages,
            tools,
            self.endpoint_kind,
            self.reasoning_effort.as_deref(),
            self.thinking,
            metadata.clone(),
        );
        body["stream"] = json!(true);

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

        let mut stream = res.bytes_stream().eventsource();
        let mut streamed_text = String::new();
        let mut streamed_reasoning_content = String::new();
        let mut streamed_tool_calls: Vec<Value> = Vec::new();
        let mut stop_reason = None;
        let mut usage = None;

        while let Some(event) =
            next_stream_item_with_idle_timeout(&mut stream, "OpenAI-compatible SSE").await?
        {
            metadata.ensure_not_cancelled()?;
            let event = event.map_err(|error| anyhow!("Failed to decode SSE event: {error}"))?;
            let data = event.data.trim();
            if data.is_empty() {
                continue;
            }
            if data == "[DONE]" {
                break;
            }
            let payload: Value = serde_json::from_str(data)
                .map_err(|error| anyhow!("Failed to parse SSE payload: {error}"))?;

            if let Some(choice) = payload
                .get("choices")
                .and_then(Value::as_array)
                .and_then(|choices| choices.first())
            {
                if let Some(delta) = choice.get("delta") {
                    if let Some(content) = delta.get("content").and_then(Value::as_str) {
                        if !content.is_empty() {
                            on_text_delta(content.to_string());
                            streamed_text.push_str(content);
                        }
                    }
                    if let Some(reasoning) = delta.get("reasoning_content").and_then(Value::as_str)
                    {
                        streamed_reasoning_content.push_str(reasoning);
                    }
                    if let Some(tool_deltas) = delta.get("tool_calls").and_then(Value::as_array) {
                        merge_streaming_tool_calls(&mut streamed_tool_calls, tool_deltas)?;
                    }
                }
                if let Some(finish) = choice.get("finish_reason").and_then(Value::as_str) {
                    stop_reason = Some(finish.to_string());
                }
            }
            if let Some(u) = payload.get("usage") {
                if !u.is_null() {
                    usage = Some(u.clone());
                }
            }
        }

        let content = build_streaming_response_content(
            self.endpoint_kind,
            streamed_text,
            streamed_reasoning_content,
            &streamed_tool_calls,
        )?;

        Ok(LlmResponse {
            content,
            stop_reason,
            usage: usage.map(|u| TokenUsage {
                input_tokens: u["prompt_tokens"].as_u64().unwrap_or(0) as u32,
                output_tokens: u["completion_tokens"].as_u64().unwrap_or(0) as u32,
            }),
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
        let body = build_chat_completion_request_body(
            &self.model,
            &msgs,
            &[],
            self.endpoint_kind,
            self.reasoning_effort.as_deref(),
            self.thinking,
            LlmTurnMetadata::default(),
        );
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

pub(super) fn build_chat_completion_request_body(
    model: &str,
    messages: &[Message],
    tools: &[Value],
    endpoint_kind: OpenAiEndpointKind,
    reasoning_effort: Option<&str>,
    thinking: Option<bool>,
    metadata: LlmTurnMetadata,
) -> Value {
    let openai_messages = to_openai_messages_for_endpoint(messages, endpoint_kind);
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

    let mut body = json!({ "model": model, "messages": openai_messages });
    if !openai_tools.is_empty() {
        body["tools"] = json!(openai_tools);
    }
    let strong_reasoning = !openai_tools.is_empty() || metadata.prefers_strong_reasoning();
    normalize_deepseek_thinking_tool_history(&mut body, model, endpoint_kind, thinking);
    apply_deepseek_thinking_options(
        &mut body,
        model,
        endpoint_kind,
        reasoning_effort,
        thinking,
        strong_reasoning,
    );
    body
}

fn normalize_deepseek_thinking_tool_history(
    body: &mut Value,
    model: &str,
    endpoint_kind: OpenAiEndpointKind,
    thinking: Option<bool>,
) {
    if endpoint_kind != OpenAiEndpointKind::Deepseek
        || !deepseek_supports_thinking(model)
        || matches!(thinking, Some(false))
    {
        return;
    }

    let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    for message in messages {
        if message.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let has_tool_calls = message
            .get("tool_calls")
            .and_then(Value::as_array)
            .is_some_and(|tool_calls| !tool_calls.is_empty());
        if has_tool_calls && message.get("reasoning_content").is_none() {
            message["reasoning_content"] = Value::String(String::new());
        }
    }
}

fn apply_deepseek_thinking_options(
    body: &mut Value,
    model: &str,
    endpoint_kind: OpenAiEndpointKind,
    reasoning_effort: Option<&str>,
    thinking: Option<bool>,
    strong_reasoning: bool,
) {
    if endpoint_kind != OpenAiEndpointKind::Deepseek || !deepseek_supports_thinking(model) {
        return;
    }

    // Only send thinking controls when the user explicitly opts in, keeping
    // the default body compatible with standard OpenAI-style endpoints.
    match thinking {
        Some(true) => {
            body["thinking"] = json!({ "type": "enabled" });
            body["reasoning_effort"] = Value::String(normalize_deepseek_reasoning_effort(
                reasoning_effort,
                strong_reasoning,
            ));
        }
        Some(false) => {
            body["thinking"] = json!({ "type": "disabled" });
        }
        None => {}
    }
}

fn deepseek_supports_thinking(model: &str) -> bool {
    let model = model.to_ascii_lowercase();
    model.contains("v4") || model.contains("reasoner")
}

fn normalize_deepseek_reasoning_effort(
    reasoning_effort: Option<&str>,
    strong_reasoning: bool,
) -> String {
    let Some(reasoning_effort) = reasoning_effort
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return if strong_reasoning { "max" } else { "high" }.to_string();
    };

    match reasoning_effort.to_ascii_lowercase().as_str() {
        "max" | "xhigh" => "max".to_string(),
        "low" | "medium" | "high" => "high".to_string(),
        _ => {
            if strong_reasoning {
                "max".to_string()
            } else {
                "high".to_string()
            }
        }
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
    let mut parsed_dsml_tool_calls = Vec::new();
    if let Some(text) = extract_message_text(choice.get("content")) {
        if endpoint_kind == OpenAiEndpointKind::Deepseek {
            let (visible_text, dsml_tool_calls) = extract_dsml_tool_calls_from_text(&text);
            parsed_dsml_tool_calls = dsml_tool_calls;
            if !visible_text.trim().is_empty() {
                content.push(ContentBlock::Text { text: visible_text });
            }
        } else if !text.trim().is_empty() {
            content.push(ContentBlock::Text { text });
        }
    }
    if endpoint_kind == OpenAiEndpointKind::Deepseek {
        if let Some(reasoning_content) = choice
            .get("reasoning_content")
            .and_then(Value::as_str)
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
    if endpoint_kind == OpenAiEndpointKind::Deepseek
        && !parsed_dsml_tool_calls.is_empty()
        && !content
            .iter()
            .any(|block| matches!(block, ContentBlock::ToolUse { .. }))
    {
        content.extend(parsed_dsml_tool_calls);
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

fn extract_dsml_tool_calls_from_text(text: &str) -> (String, Vec<ContentBlock>) {
    const TOOL_CALLS_OPEN: &str = "<｜DSML｜tool_calls>";
    const TOOL_CALLS_CLOSE: &str = "</｜DSML｜tool_calls>";

    let mut visible = String::new();
    let mut rest = text;
    let mut tool_calls = Vec::new();
    while let Some(start) = rest.find(TOOL_CALLS_OPEN) {
        visible.push_str(&rest[..start]);
        let block = &rest[start + TOOL_CALLS_OPEN.len()..];
        let Some(end) = block.find(TOOL_CALLS_CLOSE) else {
            break;
        };
        tool_calls.extend(parse_dsml_tool_call_block(&block[..end], tool_calls.len()));
        rest = &block[end + TOOL_CALLS_CLOSE.len()..];
    }
    visible.push_str(rest);

    if tool_calls.is_empty() {
        (text.to_string(), tool_calls)
    } else {
        (visible, tool_calls)
    }
}

fn parse_dsml_tool_call_block(block: &str, start_idx: usize) -> Vec<ContentBlock> {
    const INVOKE_OPEN: &str = "<｜DSML｜invoke";
    const INVOKE_CLOSE: &str = "</｜DSML｜invoke>";

    let mut calls = Vec::new();
    let mut rest = block;
    while let Some(start) = rest.find(INVOKE_OPEN) {
        let invoke = &rest[start..];
        let Some(open_end) = invoke.find('>') else {
            break;
        };
        let tag = &invoke[..=open_end];
        let Some(name) = extract_dsml_attr(tag, "name") else {
            rest = &invoke[open_end + 1..];
            continue;
        };
        let body = &invoke[open_end + 1..];
        let Some(close) = body.find(INVOKE_CLOSE) else {
            break;
        };
        let input = parse_dsml_parameters(&body[..close]);
        calls.push(ContentBlock::ToolUse {
            id: format!("dsml-tool-{}", start_idx + calls.len() + 1),
            name,
            input,
        });
        rest = &body[close + INVOKE_CLOSE.len()..];
    }
    calls
}

fn parse_dsml_parameters(body: &str) -> Value {
    const PARAM_OPEN: &str = "<｜DSML｜parameter";
    const PARAM_CLOSE: &str = "</｜DSML｜parameter>";

    let mut params = serde_json::Map::new();
    let mut rest = body;
    while let Some(start) = rest.find(PARAM_OPEN) {
        let param = &rest[start..];
        let Some(open_end) = param.find('>') else {
            break;
        };
        let tag = &param[..=open_end];
        let Some(name) = extract_dsml_attr(tag, "name") else {
            rest = &param[open_end + 1..];
            continue;
        };
        let value_body = &param[open_end + 1..];
        let Some(close) = value_body.find(PARAM_CLOSE) else {
            break;
        };
        let raw_value = value_body[..close].trim();
        let value = if tag.contains("string=\"true\"") {
            Value::String(raw_value.to_string())
        } else {
            serde_json::from_str(raw_value).unwrap_or_else(|_| Value::String(raw_value.to_string()))
        };
        params.insert(name, value);
        rest = &value_body[close + PARAM_CLOSE.len()..];
    }
    Value::Object(params)
}

fn extract_dsml_attr(tag: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let start = tag.find(&needle)? + needle.len();
    let value = &tag[start..];
    let end = value.find('"')?;
    Some(value[..end].to_string())
}

pub(super) fn build_streaming_response_content(
    endpoint_kind: OpenAiEndpointKind,
    streamed_text: String,
    streamed_reasoning_content: String,
    streamed_tool_calls: &[Value],
) -> Result<Vec<ContentBlock>> {
    let mut content = Vec::new();
    let mut parsed_dsml_tool_calls = Vec::new();

    if endpoint_kind == OpenAiEndpointKind::Deepseek {
        let (visible_text, dsml_tool_calls) = extract_dsml_tool_calls_from_text(&streamed_text);
        parsed_dsml_tool_calls = dsml_tool_calls;
        if !visible_text.trim().is_empty() {
            content.push(ContentBlock::Text { text: visible_text });
        }
    } else if !streamed_text.trim().is_empty() {
        content.push(ContentBlock::Text {
            text: streamed_text,
        });
    }

    if endpoint_kind == OpenAiEndpointKind::Deepseek && !streamed_reasoning_content.is_empty() {
        content.push(ContentBlock::ProviderMetadata {
            provider: "deepseek".to_string(),
            key: "reasoning_content".to_string(),
            value: Value::String(streamed_reasoning_content),
        });
    }

    for (idx, tc) in streamed_tool_calls.iter().enumerate() {
        let id = tc
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .map(|id| id.to_string())
            .unwrap_or_else(|| format!("stream-tool-{}", idx + 1));
        let name = tc
            .get("function")
            .and_then(|f| f.get("name"))
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .ok_or_else(|| anyhow!("OpenAI-compatible stream tool_calls[{idx}] missing name"))?;
        let arguments = tc
            .get("function")
            .and_then(|f| f.get("arguments"))
            .unwrap_or(&Value::Null);
        content.push(ContentBlock::ToolUse {
            id,
            name: name.to_string(),
            input: parse_tool_arguments(arguments)?,
        });
    }

    if endpoint_kind == OpenAiEndpointKind::Deepseek
        && !parsed_dsml_tool_calls.is_empty()
        && !content
            .iter()
            .any(|block| matches!(block, ContentBlock::ToolUse { .. }))
    {
        content.extend(parsed_dsml_tool_calls);
    }

    Ok(content)
}

pub(super) fn merge_streaming_tool_calls(
    accumulated: &mut Vec<Value>,
    deltas: &[Value],
) -> Result<()> {
    for (delta_idx, delta) in deltas.iter().enumerate() {
        let index = delta.get("index").and_then(Value::as_u64).ok_or_else(|| {
            anyhow!("OpenAI-compatible stream tool_calls[{delta_idx}] missing index")
        })? as usize;
        while accumulated.len() <= index {
            accumulated.push(json!({}));
        }
        let existing = &mut accumulated[index];

        if let Some(id) = delta.get("id").and_then(Value::as_str) {
            if !id.is_empty() {
                existing["id"] = json!(id);
            }
        }
        if let Some(type_) = delta.get("type").and_then(Value::as_str) {
            existing["type"] = json!(type_);
        }
        if let Some(function) = delta.get("function") {
            if !existing.get("function").is_some_and(Value::is_object) {
                existing["function"] = json!({});
            }
            let function_obj = existing["function"]
                .as_object_mut()
                .expect("streaming tool call function must be an object");
            if let Some(name) = function.get("name").and_then(Value::as_str) {
                if !name.is_empty() {
                    function_obj.insert("name".to_string(), json!(name));
                }
            }
            if let Some(arguments) = function.get("arguments").and_then(Value::as_str) {
                let existing_args = function_obj
                    .get("arguments")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                function_obj.insert(
                    "arguments".to_string(),
                    json!(format!("{existing_args}{arguments}")),
                );
            }
        }
    }

    Ok(())
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
