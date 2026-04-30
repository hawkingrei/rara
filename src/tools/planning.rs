use crate::tool::{Tool, ToolError};
use async_trait::async_trait;
use serde_json::{Value, json};

pub struct EnterPlanModeTool;
pub struct ExitPlanModeTool;

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
                "Call exit_plan_mode after the proposed plan is complete and ready for approval.",
                "Use <request_user_input> only when a blocking decision needs user input.",
                "Use <continue_inspection/> only when another read-only inspection pass is required."
            ]
        }))
    }
}

#[async_trait]
impl Tool for ExitPlanModeTool {
    fn name(&self) -> &str {
        "exit_plan_mode"
    }

    fn description(&self) -> &str {
        "Submit the completed proposed plan for user approval before implementation begins"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
        })
    }

    async fn call(&self, _input: Value) -> Result<Value, ToolError> {
        Ok(json!({
            "status": "exited_plan_mode",
            "instructions": [
                "Wait for the user's structured plan decision.",
                "If approved, continue in execute mode.",
                "If rejected or continued, refine the plan and call exit_plan_mode again."
            ]
        }))
    }
}
