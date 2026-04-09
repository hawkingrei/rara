use crate::tool::{Tool, ToolError};
use async_trait::async_trait;
use serde_json::{Value, json};

pub struct WebFetchTool;
#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str { "web_fetch" }
    fn description(&self) -> &str { "Fetch web content" }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "url": { "type": "string" } },
            "required": ["url"]
        })
    }
    async fn call(&self, i: Value) -> Result<Value, ToolError> {
        let url = i["url"].as_str().ok_or(ToolError::InvalidInput("url".into()))?;
        let client = reqwest::Client::new();
        let res = client.get(url).header("User-Agent", "RARA/0.1.0").send().await.map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
        let body = res.text().await.map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
        Ok(json!({ "content": body }))
    }
}
