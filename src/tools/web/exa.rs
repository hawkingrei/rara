use serde::Serialize;
use serde_json::{Value, json};
use std::time::Duration;

use crate::redaction::redact_secrets;
use crate::tool::ToolError;

const EXA_MCP_URL: &str = "https://mcp.exa.ai/mcp";
const DEFAULT_TIMEOUT_SECS: u64 = 25;

#[derive(Clone, Debug)]
pub(super) struct ExaMcpClient {
    client: reqwest::Client,
    endpoint: String,
    api_key: Option<String>,
}

impl ExaMcpClient {
    pub(super) fn from_env() -> Self {
        let api_key = std::env::var("EXA_API_KEY")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(|value| value.trim().to_string());
        Self::new(EXA_MCP_URL, api_key)
    }

    fn new(endpoint: impl Into<String>, api_key: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            endpoint: endpoint.into(),
            api_key,
        }
    }

    pub(super) async fn call<T>(&self, tool: &str, arguments: &T) -> Result<String, ToolError>
    where
        T: Serialize + Sync,
    {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": tool,
                "arguments": arguments,
            },
        });
        let endpoint = self.request_endpoint()?;

        let response = self
            .client
            .post(endpoint)
            .header("Accept", "application/json, text/event-stream")
            .json(&request)
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .send()
            .await
            .map_err(|err| {
                ToolError::ExecutionFailed(redact_secrets(format!("Exa MCP request failed: {err}")))
            })?;

        let status = response.status();
        let body = response.text().await.map_err(|err| {
            ToolError::ExecutionFailed(redact_secrets(format!("Exa MCP response failed: {err}")))
        })?;

        if !status.is_success() {
            return Err(ToolError::ExecutionFailed(format!(
                "Exa MCP returned HTTP {status}: {}",
                truncate_error(&body)
            )));
        }

        parse_mcp_response_text(&body).ok_or_else(|| {
            ToolError::ExecutionFailed("Exa MCP response did not include text content".to_string())
        })
    }

    fn request_endpoint(&self) -> Result<reqwest::Url, ToolError> {
        let mut url = reqwest::Url::parse(&self.endpoint).map_err(|err| {
            ToolError::ExecutionFailed(format!("invalid Exa MCP endpoint: {err}"))
        })?;
        if let Some(api_key) = &self.api_key {
            url.query_pairs_mut().append_pair("exaApiKey", api_key);
        }
        Ok(url)
    }
}

fn truncate_error(input: &str) -> String {
    let trimmed = input.trim();
    let mut rendered = trimmed.chars().take(500).collect::<String>();
    if rendered.chars().count() < trimmed.chars().count() {
        rendered.push_str("...");
    }
    rendered
}

pub(super) fn parse_mcp_response_text(body: &str) -> Option<String> {
    parse_json_rpc_text(body).or_else(|| parse_sse_text(body))
}

fn parse_sse_text(body: &str) -> Option<String> {
    for line in body.lines().map(str::trim) {
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        if let Some(text) = parse_json_rpc_text(data) {
            return Some(text);
        }
    }
    None
}

fn parse_json_rpc_text(input: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(input).ok()?;
    value
        .pointer("/result/content/0/text")
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

#[cfg(test)]
mod tests {
    use super::{ExaMcpClient, parse_mcp_response_text};

    #[test]
    fn parses_sse_mcp_text_response() {
        let body = r#"event: message
data: {"result":{"content":[{"type":"text","text":"search result"}]}}
"#;

        assert_eq!(
            parse_mcp_response_text(body).as_deref(),
            Some("search result")
        );
    }

    #[test]
    fn parses_plain_json_mcp_text_response() {
        let body = r#"{"result":{"content":[{"type":"text","text":"json result"}]}}"#;

        assert_eq!(
            parse_mcp_response_text(body).as_deref(),
            Some("json result")
        );
    }

    #[test]
    fn ignores_empty_sse_events() {
        let body = "data: [DONE]\n\n";

        assert_eq!(parse_mcp_response_text(body), None);
    }

    #[test]
    fn request_endpoint_adds_exa_key_only_when_needed() {
        let client = ExaMcpClient::new("https://mcp.exa.ai/mcp", Some("secret value".to_string()));
        let endpoint = client.request_endpoint().expect("endpoint");

        assert_eq!(endpoint.scheme(), "https");
        assert!(endpoint.as_str().contains("exaApiKey=secret+value"));
    }
}
