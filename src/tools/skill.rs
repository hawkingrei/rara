use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::skill::SkillManager;
use crate::tool::{Tool, ToolError};

pub struct SkillTool {
    pub skill_manager: Arc<SkillManager>,
}
#[async_trait]
impl Tool for SkillTool {
    fn name(&self) -> &str {
        "skill"
    }
    fn description(&self) -> &str {
        "Manage and invoke reusable skills"
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["list", "invoke"] },
                "skill_name": { "type": "string" }
            },
            "required": ["action"]
        })
    }
    async fn call(&self, i: Value) -> Result<Value, ToolError> {
        let action = i["action"]
            .as_str()
            .ok_or(ToolError::InvalidInput("action".into()))?;
        match action {
            "list" => Ok(json!({ "skills": self.skill_manager.list_summaries() })),
            "invoke" => {
                let name = i["skill_name"]
                    .as_str()
                    .ok_or(ToolError::InvalidInput("name".into()))?;
                let instructions = self
                    .skill_manager
                    .invoke_instructions(name)
                    .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
                Ok(json!({ "instructions": instructions }))
            }
            _ => Err(ToolError::InvalidInput("Invalid action".into())),
        }
    }
}
