use crate::tool::{ToolManager};
use crate::tool_result::{default_tool_result_store_dir, repair_tool_result_history, ToolResultStore};
use crate::llm::LlmBackend;
use crate::vectordb::{VectorDB, MemoryMetadata};
use crate::session::SessionManager;
use crate::workspace::WorkspaceMemory;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use anyhow::{Result};
use std::sync::Arc;
use uuid::Uuid;

const MAX_TOOL_ROUNDS_PER_TURN: usize = 8;
const TOOL_CONTINUATION_PROMPT: &str =
    "Tool results are now available. Continue the task. Use another tool if needed, otherwise provide the final answer. Do not repeat the tool result verbatim.";

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Message {
    pub role: String,
    pub content: Value,
}

#[derive(Debug, Deserialize)]
pub struct AnthropicResponse {
    pub content: Vec<ContentBlock>,
    pub stop_reason: Option<String>,
    pub usage: Option<TokenUsage>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentOutputMode {
    Terminal,
    Silent,
}

#[derive(Debug, Clone)]
pub enum AgentEvent {
    Status(String),
    AssistantText(String),
    ToolUse { name: String, input: Value },
    ToolResult {
        name: String,
        content: String,
        is_error: bool,
    },
}

#[derive(Debug)]
struct ToolCall {
    id: String,
    name: String,
    input: Value,
}

#[derive(Debug)]
struct TurnOutput {
    assistant_message: Message,
    tool_calls: Vec<ToolCall>,
}

pub struct Agent {
    pub tool_manager: ToolManager,
    pub llm_backend: Arc<dyn LlmBackend>,
    pub vdb: Arc<VectorDB>,
    pub session_manager: Arc<SessionManager>,
    pub workspace: Arc<WorkspaceMemory>,
    pub history: Vec<Message>,
    pub session_id: String,
    pub total_input_tokens: u32,
    pub total_output_tokens: u32,
    pub tool_result_store: ToolResultStore,
}

impl Agent {
    pub fn new(
        tool_manager: ToolManager, 
        llm_backend: Arc<dyn LlmBackend>,
        vdb: Arc<VectorDB>,
        session_manager: Arc<SessionManager>,
        workspace: Arc<WorkspaceMemory>,
    ) -> Self {
        Self {
            tool_manager,
            llm_backend,
            vdb,
            session_manager,
            workspace,
            history: Vec::new(),
            session_id: Uuid::new_v4().to_string(),
            total_input_tokens: 0,
            total_output_tokens: 0,
            tool_result_store: ToolResultStore::new(default_tool_result_store_dir())
                .expect("tool result store"),
        }
    }

    pub fn build_system_prompt(&self) -> String {
        let mut prompt = "You are RARA, an autonomous Rust-based AI agent.\n".to_string();
        let instructions = self.workspace.discover_instructions();
        if !instructions.is_empty() {
            prompt.push_str("\n## Project Instructions:\n");
            for inst in instructions { prompt.push_str(&inst); prompt.push_str("\n"); }
        }
        if let Some(mem) = self.workspace.read_memory_file() {
            prompt.push_str("\n## Local Project Memory:\n");
            prompt.push_str(&mem);
            prompt.push_str("\n");
        }
        prompt.push_str("\n## Capabilities:\n\
            - Prefer 'apply_patch' for editing existing files and use 'write_file' only for new files or full rewrites.\n\
            - Use 'remember_experience' for global vector memory.\n\
            - Use 'update_project_memory' to record facts into memory.md.\n\
            - Use 'retrieve_session_context' to recall past conversations.\n\
            - Use 'spawn_agent' or 'team_create' for complex parallel tasks.\n\
            - You are already inside the user's workspace and can inspect local files yourself.\n\
            - Do not ask the user to paste local file contents or name local files when tools can read them directly.\n\
            - For repository review or architecture analysis, inspect the workspace proactively with tools before asking follow-up questions.\n\
            - All text outside tool calls is shown directly to the user, so keep it short and useful.\n\
            - Before the first tool call, briefly state what you are about to inspect or change.\n\
            - While working, only send short progress updates at meaningful milestones.\n\
            - Read relevant code before proposing changes to it.\n\
            - Do not add features, refactors, configurability, comments, or abstractions beyond what the task requires.\n\
            - Prefer editing existing files over creating new files unless a new file is clearly necessary.\n\
            - Report outcomes faithfully. If something is not verified or not completed, say so plainly.\n\
            - When a tool is needed, emit the tool call directly.\n\
            - Do not announce a future tool call in prose.\n\
            - Do not say that you will use a tool such as 'list_files' or 'read_file'; actually call the tool instead.\n\
            - Before the first tool call, a single short sentence of intent is enough. Do not narrate every step.\n\
            - After every tool result, decide the next step immediately: either call another tool or provide the final answer.\n\
            - Do not stop at an intermediate status update once tool results are available.");
        prompt
    }

    pub async fn compact_if_needed(&mut self) -> Result<()> {
        self.compact_if_needed_with_reporter(|_| {}).await
    }

    pub async fn compact_if_needed_with_reporter<F>(&mut self, mut report: F) -> Result<()>
    where
        F: FnMut(AgentEvent),
    {
        let bpe = tiktoken_rs::cl100k_base().unwrap();
        let current_tokens: usize = self.history.iter().map(|m| {
            bpe.encode_with_special_tokens(&m.content.to_string()).len()
        }).sum();

        if current_tokens > 10000 {
            report(AgentEvent::Status(
                "Compacting long conversation history.".to_string(),
            ));
            let split_idx = (self.history.len() as f64 * 0.8) as usize;
            let summary = self.llm_backend.summarize(&self.history[..split_idx]).await?;
            let mut new_history = vec![Message {
                role: "system".to_string(),
                content: json!(format!("SUMMARY OF PREVIOUS CONVERSATION: {}", summary)),
            }];
            new_history.extend_from_slice(&self.history[split_idx..]);
            self.history = new_history;
        }
        Ok(())
    }

    pub async fn query(&mut self, prompt: String) -> Result<()> {
        self.query_with_mode(prompt, AgentOutputMode::Terminal).await
    }

    pub async fn query_with_mode(&mut self, prompt: String, output_mode: AgentOutputMode) -> Result<()> {
        self.query_with_mode_and_events(prompt, output_mode, |_| {}).await
    }

    pub async fn query_with_mode_and_events<F>(
        &mut self,
        prompt: String,
        output_mode: AgentOutputMode,
        mut report: F,
    ) -> Result<()>
    where
        F: FnMut(AgentEvent),
    {
        let turn_start_idx = self.history.len();
        let mut tool_rounds = 0usize;
        self.compact_if_needed_with_reporter(&mut report).await?;
        self.history = repair_tool_result_history(&self.history);
        
        self.history.push(Message {
            role: "user".to_string(),
            content: json!([{"type": "text", "text": prompt.clone()}]),
        });

        loop {
            let turn_output = self
                .run_model_turn(output_mode, &mut report)
                .await?;
            self.history.push(turn_output.assistant_message);

            if turn_output.tool_calls.is_empty() {
                break;
            }
            tool_rounds += 1;
            if tool_rounds > MAX_TOOL_ROUNDS_PER_TURN {
                return Err(anyhow::anyhow!(
                    "Tool loop exceeded {} rounds without reaching a final answer",
                    MAX_TOOL_ROUNDS_PER_TURN
                ));
            }

            let tool_results = self
                .execute_tool_calls(turn_output.tool_calls, &mut report)
                .await?;
            self.extend_history_for_next_turn(tool_results, &mut report);
        }

        self.session_manager.save_session(&self.session_id, &self.history)?;
        let turn_text = format!("User: {}\nAgent Response: {:?}", prompt, self.history.last().unwrap().content);
        if let Ok(vector) = self.llm_backend.embed(&turn_text).await {
            let _ = self.vdb.upsert_turn("conversations", MemoryMetadata {
                session_id: self.session_id.clone(),
                turn_index: turn_start_idx as u32,
                text: turn_text,
            }, vector).await;
        }
        Ok(())
    }

    async fn run_model_turn<F>(
        &mut self,
        output_mode: AgentOutputMode,
        report: &mut F,
    ) -> Result<TurnOutput>
    where
        F: FnMut(AgentEvent),
    {
        report(AgentEvent::Status("Sending prompt to model.".to_string()));
        let mut messages = self.history.clone();
        messages.insert(
            0,
            Message {
                role: "system".to_string(),
                content: json!(self.build_system_prompt()),
            },
        );

        let response = self
            .llm_backend
            .ask(&messages, &self.tool_manager.get_schemas())
            .await?;

        if let Some(usage) = &response.usage {
            self.total_input_tokens += usage.input_tokens;
            self.total_output_tokens += usage.output_tokens;
        }

        let mut tool_calls = Vec::new();
        for block in &response.content {
            match block {
                ContentBlock::Text { text } => {
                    report(AgentEvent::AssistantText(text.clone()));
                    if matches!(output_mode, AgentOutputMode::Terminal) {
                        println!("Agent: {}", text);
                    }
                }
                ContentBlock::ToolUse { id, name, input } => {
                    report(AgentEvent::ToolUse {
                        name: name.clone(),
                        input: input.clone(),
                    });
                    tool_calls.push(ToolCall {
                        id: id.clone(),
                        name: name.clone(),
                        input: input.clone(),
                    });
                }
            }
        }

        Ok(TurnOutput {
            assistant_message: Message {
                role: "assistant".to_string(),
                content: serde_json::to_value(&response.content)?,
            },
            tool_calls,
        })
    }

    async fn execute_tool_calls<F>(
        &mut self,
        tool_calls: Vec<ToolCall>,
        report: &mut F,
    ) -> Result<Vec<Message>>
    where
        F: FnMut(AgentEvent),
    {
        let mut tool_results = Vec::new();
        for tool_call in tool_calls {
            if let Some(tool) = self.tool_manager.get_tool(&tool_call.name) {
                report(AgentEvent::Status(format!(
                    "Running tool {}.",
                    tool_call.name
                )));
                match tool.call(tool_call.input.clone()).await {
                    Ok(result) => {
                        let result_text = self.tool_result_store.compact_result(
                            &tool_call.name,
                            &tool_call.id,
                            &tool_call.input,
                            &result,
                        )?;
                        report(AgentEvent::ToolResult {
                            name: tool_call.name.clone(),
                            content: result_text.clone(),
                            is_error: false,
                        });
                        tool_results.push(tool_result_message(
                            &tool_call.id,
                            result_text,
                            false,
                        ));
                    }
                    Err(e) => {
                        let error_text = format!("Error: {}", e);
                        report(AgentEvent::ToolResult {
                            name: tool_call.name.clone(),
                            content: error_text.clone(),
                            is_error: true,
                        });
                        tool_results.push(tool_result_message(
                            &tool_call.id,
                            error_text,
                            true,
                        ));
                    }
                }
            }
        }
        Ok(tool_results)
    }

    fn extend_history_for_next_turn<F>(&mut self, tool_results: Vec<Message>, report: &mut F)
    where
        F: FnMut(AgentEvent),
    {
        self.history.extend(tool_results);
        report(AgentEvent::Status(
            "Tool results recorded. Continuing agent reasoning.".to_string(),
        ));
        self.history.push(tool_continuation_message());
    }
}

fn tool_continuation_message() -> Message {
    Message {
        role: "user".to_string(),
        content: json!([{"type": "text", "text": TOOL_CONTINUATION_PROMPT}]),
    }
}

fn tool_result_message(tool_use_id: &str, content: String, is_error: bool) -> Message {
    let mut block = json!({
        "type": "tool_result",
        "tool_use_id": tool_use_id,
        "content": content,
    });
    if is_error {
        block["is_error"] = json!(true);
    }
    Message {
        role: "user".to_string(),
        content: json!([block]),
    }
}

#[cfg(test)]
mod tests {
    use super::{tool_continuation_message, Agent, AnthropicResponse, ContentBlock, Message, TokenUsage};
    use crate::llm::LlmBackend;
    use crate::session::SessionManager;
    use crate::tool::{Tool, ToolError, ToolManager};
    use crate::vectordb::VectorDB;
    use crate::workspace::WorkspaceMemory;
    use anyhow::Result;
    use async_trait::async_trait;
    use serde_json::{json, Value};
    use std::sync::{Arc, Mutex};

    struct StubTool;

    #[async_trait]
    impl Tool for StubTool {
        fn name(&self) -> &str { "stub_tool" }
        fn description(&self) -> &str { "Return a simple structured result" }
        fn input_schema(&self) -> Value { json!({"type":"object"}) }
        async fn call(&self, _input: Value) -> Result<Value, ToolError> {
            Ok(json!({ "status": "ok", "value": 42 }))
        }
    }

    struct SequencedBackend {
        responses: Mutex<Vec<AnthropicResponse>>,
        observed_messages: Mutex<Vec<Vec<Message>>>,
    }

    impl SequencedBackend {
        fn new(responses: Vec<AnthropicResponse>) -> Self {
            Self {
                responses: Mutex::new(responses),
                observed_messages: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl LlmBackend for SequencedBackend {
        async fn ask(&self, messages: &[Message], _tools: &[Value]) -> Result<AnthropicResponse> {
            self.observed_messages.lock().expect("lock").push(messages.to_vec());
            Ok(self.responses.lock().expect("lock").remove(0))
        }

        async fn embed(&self, _text: &str) -> Result<Vec<f32>> {
            Ok(vec![0.0; 8])
        }

        async fn summarize(&self, _messages: &[Message]) -> Result<String> {
            Ok("summary".to_string())
        }
    }

    #[tokio::test]
    async fn appends_continuation_after_tool_result() {
        let backend = Arc::new(SequencedBackend::new(vec![
            AnthropicResponse {
                content: vec![ContentBlock::ToolUse {
                    id: "tool-1".to_string(),
                    name: "stub_tool".to_string(),
                    input: json!({}),
                }],
                stop_reason: Some("tool_use".to_string()),
                usage: Some(TokenUsage::default()),
            },
            AnthropicResponse {
                content: vec![ContentBlock::Text {
                    text: "done".to_string(),
                }],
                stop_reason: Some("end_turn".to_string()),
                usage: Some(TokenUsage::default()),
            },
        ]));

        let mut tool_manager = ToolManager::new();
        tool_manager.register(Box::new(StubTool));
        let mut agent = Agent::new(
            tool_manager,
            backend.clone(),
            Arc::new(VectorDB::new("data/lancedb")),
            Arc::new(SessionManager::new().expect("session manager")),
            Arc::new(WorkspaceMemory::new().expect("workspace memory")),
        );

        agent
            .query_with_mode("do work".to_string(), super::AgentOutputMode::Silent)
            .await
            .expect("query should succeed");

        let observed = backend.observed_messages.lock().expect("lock");
        assert_eq!(observed.len(), 2);
        let second_round = &observed[1];
        assert!(second_round.iter().any(|message| message.content == tool_continuation_message().content));
        assert!(second_round.iter().any(|message| {
            message.content.to_string().contains("tool_result")
        }));
    }

    #[tokio::test]
    async fn does_not_append_continuation_without_tools() {
        let backend = Arc::new(SequencedBackend::new(vec![AnthropicResponse {
            content: vec![ContentBlock::Text {
                text: "final".to_string(),
            }],
            stop_reason: Some("end_turn".to_string()),
            usage: Some(TokenUsage::default()),
        }]));

        let mut agent = Agent::new(
            ToolManager::new(),
            backend.clone(),
            Arc::new(VectorDB::new("data/lancedb")),
            Arc::new(SessionManager::new().expect("session manager")),
            Arc::new(WorkspaceMemory::new().expect("workspace memory")),
        );

        agent
            .query_with_mode("hello".to_string(), super::AgentOutputMode::Silent)
            .await
            .expect("query should succeed");

        let observed = backend.observed_messages.lock().expect("lock");
        assert_eq!(observed.len(), 1);
        assert!(!agent
            .history
            .iter()
            .any(|message| message.content == tool_continuation_message().content));
    }

    #[tokio::test]
    async fn errors_when_tool_loop_exceeds_limit() {
        let responses = (0..=super::MAX_TOOL_ROUNDS_PER_TURN)
            .map(|idx| AnthropicResponse {
                content: vec![ContentBlock::ToolUse {
                    id: format!("tool-{idx}"),
                    name: "stub_tool".to_string(),
                    input: json!({}),
                }],
                stop_reason: Some("tool_use".to_string()),
                usage: Some(TokenUsage::default()),
            })
            .collect::<Vec<_>>();
        let backend = Arc::new(SequencedBackend::new(responses));

        let mut tool_manager = ToolManager::new();
        tool_manager.register(Box::new(StubTool));
        let mut agent = Agent::new(
            tool_manager,
            backend,
            Arc::new(VectorDB::new("data/lancedb")),
            Arc::new(SessionManager::new().expect("session manager")),
            Arc::new(WorkspaceMemory::new().expect("workspace memory")),
        );

        let error = agent
            .query_with_mode("loop".to_string(), super::AgentOutputMode::Silent)
            .await
            .expect_err("query should fail");
        assert!(error
            .to_string()
            .contains("Tool loop exceeded"));
    }
}
