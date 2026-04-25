use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("Invalid input: {0}")]
    InvalidInput(String),
    #[error("Execution failed: {0}")]
    ExecutionFailed(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolOutputStream {
    Stdout,
    Stderr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolProgressEvent {
    Output {
        stream: ToolOutputStream,
        chunk: String,
    },
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> Value;
    async fn call(&self, input: Value) -> Result<Value, ToolError>;
    async fn call_with_events(
        &self,
        input: Value,
        _report: &mut (dyn FnMut(ToolProgressEvent) + Send),
    ) -> Result<Value, ToolError> {
        self.call(input).await
    }
}

pub struct ToolManager {
    tools: BTreeMap<String, Box<dyn Tool>>,
}

impl ToolManager {
    pub fn new() -> Self {
        Self {
            tools: BTreeMap::new(),
        }
    }
    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }
    pub fn get_tool(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|b| b.as_ref())
    }
    pub fn get_schemas(&self) -> Vec<Value> {
        self.get_schemas_filtered(|_| true)
    }
    pub fn get_schemas_filtered<F>(&self, mut include: F) -> Vec<Value>
    where
        F: FnMut(&str) -> bool,
    {
        self.tools
            .iter()
            .filter_map(|(name, tool)| {
                if !include(name.as_str()) {
                    return None;
                }
                Some(serde_json::json!({
                    "name": tool.name(),
                    "description": tool.description(),
                    "input_schema": tool.input_schema(),
                }))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestTool {
        name: &'static str,
    }

    #[async_trait]
    impl Tool for TestTool {
        fn name(&self) -> &str {
            self.name
        }

        fn description(&self) -> &str {
            "test tool"
        }

        fn input_schema(&self) -> Value {
            serde_json::json!({
                "type": "object",
                "properties": {},
            })
        }

        async fn call(&self, _input: Value) -> Result<Value, ToolError> {
            Ok(Value::Null)
        }
    }

    #[test]
    fn schemas_are_returned_in_stable_name_order() {
        let mut manager = ToolManager::new();
        manager.register(Box::new(TestTool { name: "zeta_tool" }));
        manager.register(Box::new(TestTool { name: "alpha_tool" }));
        manager.register(Box::new(TestTool { name: "mid_tool" }));

        let schemas = manager.get_schemas();
        let names = schemas
            .iter()
            .map(|schema| schema["name"].as_str().unwrap_or_default())
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["alpha_tool", "mid_tool", "zeta_tool"]);
    }

    #[test]
    fn filtered_schemas_preserve_stable_name_order() {
        let mut manager = ToolManager::new();
        manager.register(Box::new(TestTool { name: "zeta_tool" }));
        manager.register(Box::new(TestTool { name: "alpha_tool" }));
        manager.register(Box::new(TestTool { name: "mid_tool" }));

        let schemas = manager.get_schemas_filtered(|name| name != "mid_tool");
        let names = schemas
            .iter()
            .map(|schema| schema["name"].as_str().unwrap_or_default())
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["alpha_tool", "zeta_tool"]);
    }
}
