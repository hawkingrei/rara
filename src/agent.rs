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
    "<agent_runtime>\nphase: tool_results_available\ninstructions:\n- Continue the same task immediately.\n- Review the tool results already present in the conversation.\n- Either call the next tool directly, or provide the final answer.\n- Do not ask the user to continue.\n- Do not repeat tool results verbatim.\n</agent_runtime>";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentExecutionMode {
    Execute,
    Plan,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BashApprovalMode {
    Once,
    Always,
    Suggestion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanStepStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanStep {
    pub step: String,
    pub status: PlanStepStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingUserInput {
    pub question: String,
    pub options: Vec<(String, String)>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingApproval {
    pub tool_use_id: String,
    pub command: String,
    pub allow_net: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletedInteraction {
    pub title: String,
    pub summary: String,
}

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
    pub execution_mode: AgentExecutionMode,
    pub bash_approval_mode: BashApprovalMode,
    pub current_plan: Vec<PlanStep>,
    pub plan_explanation: Option<String>,
    pub pending_user_input: Option<PendingUserInput>,
    pub pending_approval: Option<PendingApproval>,
    pub completed_user_input: Option<CompletedInteraction>,
    pub completed_approval: Option<CompletedInteraction>,
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
            execution_mode: AgentExecutionMode::Execute,
            bash_approval_mode: BashApprovalMode::Always,
            current_plan: Vec::new(),
            plan_explanation: None,
            pending_user_input: None,
            pending_approval: None,
            completed_user_input: None,
            completed_approval: None,
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
        if matches!(self.execution_mode, AgentExecutionMode::Plan) {
            prompt.push_str(
                "\n## Current Execution Mode:\n\
                - Plan mode is active.\n\
                - This pass is read-only.\n\
                - Inspect the codebase, analyze constraints, and produce a concrete implementation plan.\n\
                - Do not call tools that edit files, run shell commands, update project memory, save experience, or spawn sub-agents.\n",
            );
            prompt.push_str(
                "- Start your response with a <plan> block.\n\
                - Inside the block, emit one step per line in the form '- [pending] Step' or '- [in_progress] Step' or '- [completed] Step'.\n\
                - After </plan>, provide a short explanation grounded in the inspected code.\n",
            );
            prompt.push_str(
                "- If a key product or implementation decision blocks progress, also emit a <request_user_input> block.\n\
                - Inside that block, write one 'question: ...' line and up to three 'option: label | description' lines.\n\
                - After </request_user_input>, keep the rest of the explanation concise.\n",
            );
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
            - For repository review, avoid repeating the same discovery tool call with the same arguments unless the workspace changed.\n\
            - Prefer source directories and key project files over build artifacts or cache directories when inspecting a repository.\n\
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
            - Do not stop at an intermediate status update once tool results are available.\n\
            - Runtime may append an <agent_runtime> block after tool execution.\n\
            - Treat that block as internal execution state, not as a new user request.\n\
            - When phase is 'tool_results_available', continue the same task immediately.");
        prompt
    }

    pub fn set_execution_mode(&mut self, mode: AgentExecutionMode) {
        self.execution_mode = mode;
    }

    pub fn execution_mode_label(&self) -> &'static str {
        match self.execution_mode {
            AgentExecutionMode::Execute => "execute",
            AgentExecutionMode::Plan => "plan",
        }
    }

    pub fn set_bash_approval_mode(&mut self, mode: BashApprovalMode) {
        self.bash_approval_mode = mode;
    }

    pub fn clear_completed_interactions(&mut self) {
        self.completed_user_input = None;
        self.completed_approval = None;
    }

    pub fn consume_pending_user_input(&mut self, answer: &str) {
        if let Some(pending) = self.pending_user_input.take() {
            self.completed_user_input = Some(CompletedInteraction {
                title: pending.question,
                summary: format!("Answered with: {}", answer.trim()),
            });
        }
    }

    pub async fn answer_pending_approval_with_events<F>(
        &mut self,
        selection: BashApprovalMode,
        output_mode: AgentOutputMode,
        mut report: F,
    ) -> Result<()>
    where
        F: FnMut(AgentEvent),
    {
        let pending = self
            .pending_approval
            .clone()
            .ok_or_else(|| anyhow::anyhow!("No pending approval to answer"))?;

        self.pending_approval = None;
        self.pending_user_input = None;
        self.completed_approval = None;

        match selection {
            BashApprovalMode::Once => {
                self.completed_approval = Some(CompletedInteraction {
                    title: "Bash approval".to_string(),
                    summary: format!("Approved once for command: {}", pending.command),
                });
                self.execute_pending_bash(pending, false, output_mode, &mut report)
                    .await?;
            }
            BashApprovalMode::Always => {
                self.bash_approval_mode = BashApprovalMode::Always;
                self.completed_approval = Some(CompletedInteraction {
                    title: "Bash approval".to_string(),
                    summary: format!("Approved for session: {}", pending.command),
                });
                self.execute_pending_bash(pending, true, output_mode, &mut report)
                    .await?;
            }
            BashApprovalMode::Suggestion => {
                self.completed_approval = Some(CompletedInteraction {
                    title: "Bash approval".to_string(),
                    summary: format!("Kept as suggestion only: {}", pending.command),
                });
                let error_text = "Bash command was not approved. Continue without shell execution and find a safer path.".to_string();
                report(AgentEvent::ToolResult {
                    name: "bash".to_string(),
                    content: error_text.clone(),
                    is_error: true,
                });
                self.history.push(tool_result_message(
                    &pending.tool_use_id,
                    error_text,
                    true,
                ));
                self.history.push(tool_continuation_message());
                self.run_agent_loop(output_mode, &mut report).await?;
            }
        }

        self.session_manager.save_session(&self.session_id, &self.history)?;
        Ok(())
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
        self.clear_completed_interactions();
        
        self.history.push(Message {
            role: "user".to_string(),
            content: json!([{"type": "text", "text": prompt.clone()}]),
        });

        self.run_agent_loop_with_limit(output_mode, &mut report, &mut tool_rounds)
            .await?;

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
            .ask(&messages, &self.visible_tool_schemas())
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
                    if matches!(self.execution_mode, AgentExecutionMode::Plan) {
                        self.capture_plan_from_text(text);
                    }
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

    async fn run_agent_loop<F>(&mut self, output_mode: AgentOutputMode, report: &mut F) -> Result<()>
    where
        F: FnMut(AgentEvent),
    {
        let mut tool_rounds = 0usize;
        self.run_agent_loop_with_limit(output_mode, report, &mut tool_rounds)
            .await
    }

    async fn run_agent_loop_with_limit<F>(
        &mut self,
        output_mode: AgentOutputMode,
        report: &mut F,
        tool_rounds: &mut usize,
    ) -> Result<()>
    where
        F: FnMut(AgentEvent),
    {
        loop {
            let turn_output = self
                .run_model_turn(output_mode, report)
                .await?;
            self.history.push(turn_output.assistant_message);

            if turn_output.tool_calls.is_empty() {
                break;
            }
            *tool_rounds += 1;
            if *tool_rounds > MAX_TOOL_ROUNDS_PER_TURN {
                return Err(anyhow::anyhow!(
                    "Tool loop exceeded {} rounds without reaching a final answer",
                    MAX_TOOL_ROUNDS_PER_TURN
                ));
            }

            let tool_results = self
                .execute_tool_calls(turn_output.tool_calls, report)
                .await?;
            if self.pending_approval.is_some() {
                break;
            }
            self.extend_history_for_next_turn(tool_results, report);
        }
        Ok(())
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
            if tool_call.name == "bash" && matches!(self.bash_approval_mode, BashApprovalMode::Suggestion) {
                let command = tool_call
                    .input
                    .get("command")
                    .and_then(Value::as_str)
                    .unwrap_or("<command>");
                let allow_net = tool_call
                    .input
                    .get("allow_net")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                self.pending_approval = Some(PendingApproval {
                    tool_use_id: tool_call.id.clone(),
                    command: command.to_string(),
                    allow_net,
                });
                self.pending_user_input = Some(PendingUserInput {
                    question: "Bash command needs approval. What should RARA do?".to_string(),
                    options: vec![
                        (
                            "Run once".to_string(),
                            "Execute this command now and then return to suggestion mode.".to_string(),
                        ),
                        (
                            "Always allow bash".to_string(),
                            "Execute now and keep bash approval open for later commands.".to_string(),
                        ),
                        (
                            "Suggestion only".to_string(),
                            "Do not run the command automatically. Continue with a safer path.".to_string(),
                        ),
                    ],
                    note: Some(format!("command: {}", command)),
                });
                report(AgentEvent::Status(
                    "Bash approval required. Waiting for a structured user decision.".to_string(),
                ));
                break;
            }
            if !self.is_tool_allowed_in_current_mode(&tool_call.name) {
                let error_text = format!(
                    "Error: tool '{}' is unavailable in {} mode. Inspect with read-only tools and return a plan instead.",
                    tool_call.name,
                    self.execution_mode_label()
                );
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
                continue;
            }
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

    async fn execute_pending_bash<F>(
        &mut self,
        pending: PendingApproval,
        keep_always: bool,
        output_mode: AgentOutputMode,
        report: &mut F,
    ) -> Result<()>
    where
        F: FnMut(AgentEvent),
    {
        let input = json!({
            "command": pending.command,
            "allow_net": pending.allow_net,
        });
        report(AgentEvent::ToolUse {
            name: "bash".to_string(),
            input: input.clone(),
        });
        let tool = self
            .tool_manager
            .get_tool("bash")
            .ok_or_else(|| anyhow::anyhow!("bash tool is unavailable"))?;
        report(AgentEvent::Status("Running approved bash command.".to_string()));
        match tool.call(input.clone()).await {
            Ok(result) => {
                let result_text = self.tool_result_store.compact_result(
                    "bash",
                    &pending.tool_use_id,
                    &input,
                    &result,
                )?;
                report(AgentEvent::ToolResult {
                    name: "bash".to_string(),
                    content: result_text.clone(),
                    is_error: false,
                });
                self.history.push(tool_result_message(
                    &pending.tool_use_id,
                    result_text,
                    false,
                ));
            }
            Err(err) => {
                let error_text = format!("Error: {}", err);
                report(AgentEvent::ToolResult {
                    name: "bash".to_string(),
                    content: error_text.clone(),
                    is_error: true,
                });
                self.history.push(tool_result_message(
                    &pending.tool_use_id,
                    error_text,
                    true,
                ));
            }
        }
        self.history.push(tool_continuation_message());
        if !keep_always {
            self.bash_approval_mode = BashApprovalMode::Suggestion;
        }
        self.run_agent_loop(output_mode, report).await
    }

    fn extend_history_for_next_turn<F>(&mut self, tool_results: Vec<Message>, report: &mut F)
    where
        F: FnMut(AgentEvent),
    {
        self.history.extend(tool_results);
        report(AgentEvent::Status(
            "Tool results recorded. Advancing to the next agent step.".to_string(),
        ));
        self.history.push(tool_continuation_message());
    }

    fn visible_tool_schemas(&self) -> Vec<Value> {
        self.tool_manager
            .get_schemas_filtered(|name| self.is_tool_allowed_in_current_mode(name))
    }

    fn is_tool_allowed_in_current_mode(&self, name: &str) -> bool {
        match self.execution_mode {
            AgentExecutionMode::Execute => true,
            AgentExecutionMode::Plan => !matches!(
                name,
                "bash"
                    | "write_file"
                    | "replace"
                    | "apply_patch"
                    | "update_project_memory"
                    | "remember_experience"
                    | "spawn_agent"
                    | "team_create"
            ),
        }
    }

    fn capture_plan_from_text(&mut self, text: &str) {
        let Some((steps, explanation)) = parse_plan_block(text) else {
            self.pending_user_input = parse_request_user_input_block(text);
            return;
        };
        if !steps.is_empty() {
            self.current_plan = steps;
        }
        self.plan_explanation = explanation;
        self.pending_user_input = parse_request_user_input_block(text);
    }
}

fn parse_plan_block(text: &str) -> Option<(Vec<PlanStep>, Option<String>)> {
    let start = text.find("<plan>")?;
    let end = text.find("</plan>")?;
    if end <= start {
        return None;
    }

    let block = &text[start + "<plan>".len()..end];
    let mut steps = Vec::new();
    for line in block.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let Some(rest) = line.strip_prefix("- [") else {
            continue;
        };
        let Some((status, step)) = rest.split_once("] ") else {
            continue;
        };
        let status = match status.trim() {
            "pending" => PlanStepStatus::Pending,
            "in_progress" => PlanStepStatus::InProgress,
            "completed" => PlanStepStatus::Completed,
            _ => continue,
        };
        steps.push(PlanStep {
            step: step.trim().to_string(),
            status,
        });
    }

    let explanation = text[end + "</plan>".len()..].trim();
    Some((
        steps,
        (!explanation.is_empty()).then(|| explanation.to_string()),
    ))
}

fn parse_request_user_input_block(text: &str) -> Option<PendingUserInput> {
    let start = text.find("<request_user_input>")?;
    let end = text.find("</request_user_input>")?;
    if end <= start {
        return None;
    }

    let block = &text[start + "<request_user_input>".len()..end];
    let mut question = None;
    let mut options = Vec::new();
    for line in block.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if let Some(value) = line.strip_prefix("question:") {
            question = Some(value.trim().to_string());
            continue;
        }
        if let Some(value) = line.strip_prefix("option:") {
            let value = value.trim();
            if let Some((label, description)) = value.split_once('|') {
                options.push((label.trim().to_string(), description.trim().to_string()));
            } else {
                options.push((value.to_string(), String::new()));
            }
        }
    }

    let note = text[end + "</request_user_input>".len()..]
        .trim()
        .strip_prefix("</plan>")
        .unwrap_or(text[end + "</request_user_input>".len()..].trim())
        .trim()
        .to_string();

    Some(PendingUserInput {
        question: question?,
        options,
        note: (!note.is_empty()).then_some(note),
    })
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
    use super::{parse_plan_block, parse_request_user_input_block, tool_continuation_message, Agent, AgentExecutionMode, AnthropicResponse, ContentBlock, Message, PendingUserInput, PlanStep, PlanStepStatus, TokenUsage};
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
        observed_tools: Mutex<Vec<Vec<String>>>,
    }

    impl SequencedBackend {
        fn new(responses: Vec<AnthropicResponse>) -> Self {
            Self {
                responses: Mutex::new(responses),
                observed_messages: Mutex::new(Vec::new()),
                observed_tools: Mutex::new(Vec::new()),
            }
        }

        fn observed_tools(&self) -> Vec<Vec<String>> {
            self.observed_tools.lock().expect("lock").clone()
        }
    }

    #[async_trait]
    impl LlmBackend for SequencedBackend {
        async fn ask(&self, messages: &[Message], tools: &[Value]) -> Result<AnthropicResponse> {
            self.observed_messages.lock().expect("lock").push(messages.to_vec());
            self.observed_tools.lock().expect("lock").push(
                tools
                    .iter()
                    .filter_map(|tool| tool.get("name").and_then(Value::as_str).map(str::to_string))
                    .collect(),
            );
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

    #[tokio::test]
    async fn plan_mode_filters_write_tools_from_schema() {
        let backend = Arc::new(SequencedBackend::new(vec![AnthropicResponse {
            content: vec![ContentBlock::Text {
                text: "plan".to_string(),
            }],
            stop_reason: Some("end_turn".to_string()),
            usage: Some(TokenUsage::default()),
        }]));

        let mut tool_manager = ToolManager::new();
        tool_manager.register(Box::new(StubTool));
        tool_manager.register(Box::new(crate::tools::file::WriteFileTool));
        let mut agent = Agent::new(
            tool_manager,
            backend.clone(),
            Arc::new(VectorDB::new("data/lancedb")),
            Arc::new(SessionManager::new().expect("session manager")),
            Arc::new(WorkspaceMemory::new().expect("workspace memory")),
        );
        agent.set_execution_mode(AgentExecutionMode::Plan);

        agent
            .query_with_mode("inspect".to_string(), super::AgentOutputMode::Silent)
            .await
            .expect("query should succeed");

        let observed_tools = backend.observed_tools();
        assert_eq!(observed_tools.len(), 1);
        assert_eq!(observed_tools[0], vec!["stub_tool".to_string()]);
    }

    #[test]
    fn parses_structured_plan_block() {
        let text = "<plan>\n- [in_progress] Inspect core agent loop\n- [pending] Review TUI rendering path\n- [completed] Confirm current constraints\n</plan>\nFocus on agent.rs and tui/runtime.rs first.";
        let parsed = parse_plan_block(text).expect("plan block should parse");
        assert_eq!(
            parsed.0,
            vec![
                PlanStep {
                    step: "Inspect core agent loop".to_string(),
                    status: PlanStepStatus::InProgress,
                },
                PlanStep {
                    step: "Review TUI rendering path".to_string(),
                    status: PlanStepStatus::Pending,
                },
                PlanStep {
                    step: "Confirm current constraints".to_string(),
                    status: PlanStepStatus::Completed,
                },
            ]
        );
        assert_eq!(
            parsed.1.as_deref(),
            Some("Focus on agent.rs and tui/runtime.rs first.")
        );
    }

    #[test]
    fn parses_request_user_input_block() {
        let text = "<request_user_input>\nquestion: Which path should we take first?\noption: Minimal | Keep the diff small and local.\noption: Broad | Reshape the module boundaries now.\n</request_user_input>\nNeed direction before editing.";
        let parsed = parse_request_user_input_block(text).expect("question block should parse");
        assert_eq!(
            parsed,
            PendingUserInput {
                question: "Which path should we take first?".to_string(),
                options: vec![
                    (
                        "Minimal".to_string(),
                        "Keep the diff small and local.".to_string(),
                    ),
                    (
                        "Broad".to_string(),
                        "Reshape the module boundaries now.".to_string(),
                    ),
                ],
                note: Some("Need direction before editing.".to_string()),
            }
        );
    }
}
