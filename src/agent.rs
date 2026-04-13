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
            - Use 'spawn_agent' or 'team_create' for complex parallel tasks.");
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
        self.compact_if_needed_with_reporter(&mut report).await?;
        self.history = repair_tool_result_history(&self.history);
        
        self.history.push(Message {
            role: "user".to_string(),
            content: json!([{"type": "text", "text": prompt.clone()}]),
        });

        loop {
            report(AgentEvent::Status("Sending prompt to model.".to_string()));
            let mut messages = self.history.clone();
            messages.insert(0, Message { role: "system".to_string(), content: json!(self.build_system_prompt()) });

            let response = self.llm_backend.ask(&messages, &self.tool_manager.get_schemas()).await?;

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
                        tool_calls.push((id.clone(), name.clone(), input.clone()));
                    }
                }
            }

            self.history.push(Message {
                role: "assistant".to_string(),
                content: serde_json::to_value(&response.content)?,
            });

            if tool_calls.is_empty() { break; }

            for (id, name, input) in tool_calls {
                if let Some(tool) = self.tool_manager.get_tool(&name) {
                    report(AgentEvent::Status(format!("Running tool {name}.")));
                    match tool.call(input.clone()).await {
                        Ok(result) => {
                            let result_text = self.tool_result_store.compact_result(
                                &name,
                                &id,
                                &input,
                                &result,
                            )?;
                            report(AgentEvent::ToolResult {
                                name: name.clone(),
                                content: result_text.clone(),
                                is_error: false,
                            });
                            self.history.push(Message {
                                role: "user".to_string(),
                                content: json!([{"type": "tool_result", "tool_use_id": id, "content": result_text}]),
                            });
                        }
                        Err(e) => {
                            let error_text = format!("Error: {}", e);
                            report(AgentEvent::ToolResult {
                                name: name.clone(),
                                content: error_text.clone(),
                                is_error: true,
                            });
                            self.history.push(Message {
                                role: "user".to_string(),
                                content: json!([{"type": "tool_result", "tool_use_id": id, "content": error_text, "is_error": true}]),
                            });
                        }
                    }
                }
            }
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
}
