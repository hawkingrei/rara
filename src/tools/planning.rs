use crate::tool::{Tool, ToolError};
use async_trait::async_trait;
use serde_json::{Value, json};

pub struct EnterPlanModeTool;

#[async_trait]
impl Tool for EnterPlanModeTool {
    fn name(&self) -> &str {
        "enter_plan_mode"
    }

    fn description(&self) -> &str {
        "Enter read-only planning mode when the task needs repository exploration and design before implementation"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
        })
    }

    async fn call(&self, _input: Value) -> Result<Value, ToolError> {
        Ok(json!({
            "status": "entered_plan_mode",
            "instructions": [
                "Inspect the repository with read-only tools.",
                "Return a normal final answer for research, review, or planning-advice tasks.",
                "Use a <proposed_plan> block only when you are requesting approval to implement a concrete plan.",
                "Use <request_user_input> only when a blocking decision needs user input.",
                "Use <continue_inspection/> only when another read-only inspection pass is required."
            ]
        }))
    }
}
