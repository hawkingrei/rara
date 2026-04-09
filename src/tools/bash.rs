use crate::tool::{Tool, ToolError};
use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::process::Command;
use crate::sandbox::SandboxManager;
use std::sync::Arc;
use std::env;

pub struct BashTool { pub sandbox: Arc<SandboxManager> }

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str { "bash" }
    fn description(&self) -> &str { "Run shell command in sandbox" }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": { "type": "string" },
                "allow_net": { "type": "boolean", "default": false }
            },
            "required": ["command"]
        })
    }
    async fn call(&self, i: Value) -> Result<Value, ToolError> {
        let cmd = i["command"].as_str().ok_or(ToolError::InvalidInput("command".into()))?;
        let net = i["allow_net"].as_bool().unwrap_or(false);
        let cwd = env::current_dir()?.to_string_lossy().to_string();
        let wcmd = self.sandbox.wrap_command(cmd, &cwd, net).map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
        let out = Command::new("bash").arg("-c").arg(&wcmd).output().await?;
        Ok(json!({
            "stdout": String::from_utf8_lossy(&out.stdout),
            "stderr": String::from_utf8_lossy(&out.stderr),
            "exit_code": out.status.code()
        }))
    }
}
