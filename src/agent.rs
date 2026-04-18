use crate::tool::{ToolManager};
use crate::tool_result::{default_tool_result_store_dir, repair_tool_result_history, ToolResultStore};
use crate::llm::{ContextBudget, LlmBackend};
use crate::vectordb::{VectorDB, MemoryMetadata};
use crate::session::SessionManager;
use crate::workspace::WorkspaceMemory;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use anyhow::{Result};
use std::sync::Arc;
use uuid::Uuid;

const MAX_TOOL_ROUNDS_PER_TURN: usize = 8;
const MAX_PLAN_CONTINUATIONS_PER_TURN: usize = 2;
const MAX_EXECUTE_CONTINUATIONS_PER_TURN: usize = 2;

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

#[derive(Debug, Clone, Default)]
pub struct CompactState {
    pub estimated_history_tokens: usize,
    pub context_window_tokens: Option<usize>,
    pub compact_threshold_tokens: usize,
    pub reserved_output_tokens: usize,
    pub compaction_count: usize,
    pub last_compaction_before_tokens: Option<usize>,
    pub last_compaction_after_tokens: Option<usize>,
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
    AssistantDelta(String),
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
    plan_updated: bool,
    assistant_text: String,
}

#[derive(Debug, Clone, Default)]
struct InspectionProgress {
    list_calls: usize,
    source_reads: usize,
    config_reads: usize,
    instruction_reads: usize,
}

#[derive(Clone, Copy)]
enum RuntimeContinuationPhase {
    ToolResultsAvailable,
    PlanContinuationRequired,
    ExecutionContinuationRequired,
}

#[derive(Serialize)]
struct RuntimeContinuation<'a> {
    phase: &'a str,
    mode: &'a str,
    tool_rounds: usize,
    inspection: RuntimeInspectionSnapshot,
    plan: RuntimePlanSnapshot,
    pending_interaction: &'a str,
    instructions: Vec<&'a str>,
}

#[derive(Serialize)]
struct RuntimeInspectionSnapshot {
    list_calls: usize,
    source_reads: usize,
    config_reads: usize,
    instruction_reads: usize,
    has_minimum_review_evidence: bool,
}

#[derive(Serialize)]
struct RuntimePlanSnapshot {
    total_steps: usize,
    pending_steps: usize,
    in_progress_steps: usize,
    completed_steps: usize,
}

impl InspectionProgress {
    fn record_tool(&mut self, name: &str, input: &Value) {
        match name {
            "list_files" => {
                self.list_calls += 1;
            }
            "read_file" => {
                let path = input
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if path.starts_with("src/") || path.ends_with(".rs") {
                    self.source_reads += 1;
                } else if path.ends_with("cargo.toml")
                    || path.ends_with("cargo.lock")
                    || path.ends_with(".toml")
                {
                    self.config_reads += 1;
                } else if path.ends_with("agents.md")
                    || path.ends_with("readme.md")
                    || path.ends_with(".rara/instructions.md")
                    || path.ends_with(".rara/memory.md")
                    || path.ends_with(".md")
                {
                    self.instruction_reads += 1;
                }
            }
            _ => {}
        }
    }

    fn has_minimum_review_evidence(&self) -> bool {
        self.source_reads >= 2
            || (self.source_reads >= 1
                && self.list_calls >= 1
                && (self.config_reads >= 1 || self.instruction_reads >= 1))
    }

    fn has_any_evidence(&self) -> bool {
        self.list_calls > 0
            || self.source_reads > 0
            || self.config_reads > 0
            || self.instruction_reads > 0
    }
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
    pub compact_state: CompactState,
    last_query_plan_updated: bool,
    inspection_progress: InspectionProgress,
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
            compact_state: CompactState::default(),
            last_query_plan_updated: false,
            inspection_progress: InspectionProgress::default(),
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
                - Use this mode to inspect the codebase, clarify constraints, and refine an implementation approach.\n\
                - Do not call tools that edit files, run shell commands, update project memory, save experience, or spawn sub-agents.\n",
            );
            prompt.push_str(
                "- Do not emit a <plan> block until the plan is decision-complete and ready for approval.\n\
                - While still exploring or refining tradeoffs, respond with short, concrete planning updates grounded in the inspected code.\n\
                - When the plan is ready for approval, start your response with a <plan> block.\n\
                - Inside the block, emit one step per line in the form '- [pending] Step' or '- [in_progress] Step' or '- [completed] Step'.\n\
                - After </plan>, provide a short explanation grounded in the inspected code.\n\
                - Keep plans shallow, concise, and grouped by behavior instead of deep trees or file-by-file inventories.\n",
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
            - For repository review or architecture analysis, keep inspecting relevant source files until you have enough concrete evidence for actionable suggestions.\n\
            - Do not stop after saying which file you want to inspect next. Call the tool for that file immediately.\n\
            - Before the first tool call, a single short sentence of intent is enough. Do not narrate every step.\n\
            - After every tool result, decide the next step immediately: either call another tool or provide the final answer.\n\
            - Do not stop at an intermediate status update once tool results are available.\n\
            - Runtime may append an <agent_runtime> block after tool execution.\n\
            - Treat that block as internal execution state, not as a new user request.\n\
            - Follow the runtime block fields and instructions directly.\n\
            - When phase is 'tool_results_available', continue the same task immediately.\n\
            - When phase is 'plan_continuation_required', keep planning in read-only mode and inspect more code before stopping.\n\
            - When phase is 'execution_continuation_required', continue the same repository inspection instead of ending early.");
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

    pub fn last_query_produced_plan(&self) -> bool {
        self.last_query_plan_updated
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
        F: FnMut(AgentEvent) + Send,
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
                self.history.push(
                    self.runtime_continuation_message(
                        RuntimeContinuationPhase::ToolResultsAvailable,
                        0,
                    ),
                );
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
        F: FnMut(AgentEvent) + Send,
    {
        self.compact_history_with_reporter(false, &mut report).await
    }

    pub async fn compact_now_with_reporter<F>(&mut self, mut report: F) -> Result<bool>
    where
        F: FnMut(AgentEvent) + Send,
    {
        self.compact_history_with_reporter(true, &mut report).await?;
        Ok(self.compact_state.last_compaction_before_tokens.is_some())
    }

    async fn compact_history_with_reporter<F>(&mut self, force: bool, report: &mut F) -> Result<()>
    where
        F: FnMut(AgentEvent) + Send,
    {
        let current_tokens = estimate_history_tokens(&self.history)?;
        let compact_budget = self.current_compact_budget();
        self.compact_state.estimated_history_tokens = current_tokens;
        self.compact_state.context_window_tokens =
            compact_budget.as_ref().map(|budget| budget.context_window_tokens);
        self.compact_state.compact_threshold_tokens =
            compact_budget.as_ref().map(|budget| budget.compact_threshold_tokens).unwrap_or(10_000);
        self.compact_state.reserved_output_tokens =
            compact_budget.as_ref().map(|budget| budget.reserved_output_tokens).unwrap_or(0);
        self.compact_state.last_compaction_before_tokens = None;
        self.compact_state.last_compaction_after_tokens = None;

        let threshold = self.compact_state.compact_threshold_tokens;
        if !force && current_tokens <= threshold {
            return Ok(());
        }
        if self.history.len() < 2 {
            return Ok(());
        }

        report(AgentEvent::Status(if force {
            "Compacting conversation history on demand.".to_string()
        } else {
            "Compacting long conversation history.".to_string()
        }));

        let split_idx = (self.history.len() as f64 * 0.8) as usize;
        let split_idx = split_idx.clamp(1, self.history.len().saturating_sub(1));
        let summary = self.llm_backend.summarize(&self.history[..split_idx]).await?;
        let mut new_history = vec![Message {
            role: "system".to_string(),
            content: json!(format!("SUMMARY OF PREVIOUS CONVERSATION: {}", summary)),
        }];
        new_history.extend_from_slice(&self.history[split_idx..]);
        self.history = new_history;
        self.session_manager.save_session(&self.session_id, &self.history)?;

        let compacted_tokens = estimate_history_tokens(&self.history)?;
        self.compact_state.estimated_history_tokens = compacted_tokens;
        self.compact_state.compaction_count += 1;
        self.compact_state.last_compaction_before_tokens = Some(current_tokens);
        self.compact_state.last_compaction_after_tokens = Some(compacted_tokens);
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
        F: FnMut(AgentEvent) + Send,
    {
        let turn_start_idx = self.history.len();
        let mut tool_rounds = 0usize;
        let mut plan_continuations = 0usize;
        let mut execute_continuations = 0usize;
        self.last_query_plan_updated = false;
        self.inspection_progress = InspectionProgress::default();
        self.compact_if_needed_with_reporter(&mut report).await?;
        self.history = repair_tool_result_history(&self.history);
        self.clear_completed_interactions();
        
        self.history.push(Message {
            role: "user".to_string(),
            content: json!([{"type": "text", "text": prompt.clone()}]),
        });

        self.run_agent_loop_with_limit(
            output_mode,
            &mut report,
            &mut tool_rounds,
            &mut plan_continuations,
            &mut execute_continuations,
        )
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
        F: FnMut(AgentEvent) + Send,
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

        let mut streamed_any_delta = false;
        let response = self
            .llm_backend
            .ask_streaming(&messages, &self.visible_tool_schemas(), &mut |delta| {
                streamed_any_delta = true;
                report(AgentEvent::AssistantDelta(delta));
            })
            .await?;

        if let Some(usage) = &response.usage {
            self.total_input_tokens += usage.input_tokens;
            self.total_output_tokens += usage.output_tokens;
        }

        let mut tool_calls = Vec::new();
        let mut plan_updated = false;
        let mut assistant_text = String::new();
        for block in &response.content {
            match block {
                ContentBlock::Text { text } => {
                    if !streamed_any_delta {
                        report(AgentEvent::AssistantText(text.clone()));
                    }
                    if !assistant_text.is_empty() {
                        assistant_text.push('\n');
                    }
                    assistant_text.push_str(text);
                    if matches!(self.execution_mode, AgentExecutionMode::Plan) {
                        plan_updated = self.capture_plan_from_text(text) || plan_updated;
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
            plan_updated,
            assistant_text,
        })
    }

    async fn run_agent_loop<F>(&mut self, output_mode: AgentOutputMode, report: &mut F) -> Result<()>
    where
        F: FnMut(AgentEvent) + Send,
    {
        let mut tool_rounds = 0usize;
        let mut plan_continuations = 0usize;
        let mut execute_continuations = 0usize;
        self.run_agent_loop_with_limit(
            output_mode,
            report,
            &mut tool_rounds,
            &mut plan_continuations,
            &mut execute_continuations,
        )
            .await
    }

    async fn run_agent_loop_with_limit<F>(
        &mut self,
        output_mode: AgentOutputMode,
        report: &mut F,
        tool_rounds: &mut usize,
        plan_continuations: &mut usize,
        execute_continuations: &mut usize,
    ) -> Result<()>
    where
        F: FnMut(AgentEvent) + Send,
    {
        loop {
            self.ensure_active_plan_step();
            let turn_output = self
                .run_model_turn(output_mode, report)
                .await?;
            self.last_query_plan_updated = turn_output.plan_updated;
            self.history.push(turn_output.assistant_message);

            if turn_output.tool_calls.is_empty() {
                if self.should_continue_plan_without_tools(
                    turn_output.plan_updated,
                    &turn_output.assistant_text,
                    *tool_rounds,
                    *plan_continuations,
                ) {
                    *plan_continuations += 1;
                    report(AgentEvent::Status(
                        "Plan needs more repository inspection. Continuing in read-only mode."
                            .to_string(),
                    ));
                    self.history.push(self.runtime_continuation_message(
                        RuntimeContinuationPhase::PlanContinuationRequired,
                        *tool_rounds,
                    ));
                    continue;
                }
                if self.should_continue_execute_without_tools(
                    &turn_output.assistant_text,
                    *tool_rounds,
                    *execute_continuations,
                ) {
                    *execute_continuations += 1;
                    report(AgentEvent::Status(
                        "Repository review needs more code inspection. Continuing the same turn."
                            .to_string(),
                    ));
                    self.history.push(self.runtime_continuation_message(
                        RuntimeContinuationPhase::ExecutionContinuationRequired,
                        *tool_rounds,
                    ));
                    continue;
                }
                self.complete_remaining_plan_steps();
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
            self.advance_plan_step();
            self.extend_history_for_next_turn(tool_results, report, *tool_rounds);
        }
        Ok(())
    }

    async fn execute_tool_calls<F>(
        &mut self,
        tool_calls: Vec<ToolCall>,
        report: &mut F,
    ) -> Result<Vec<Message>>
    where
        F: FnMut(AgentEvent) + Send,
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
                self.inspection_progress
                    .record_tool(&tool_call.name, &tool_call.input);
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
        F: FnMut(AgentEvent) + Send,
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
        self.history.push(
            self.runtime_continuation_message(RuntimeContinuationPhase::ToolResultsAvailable, 1),
        );
        if !keep_always {
            self.bash_approval_mode = BashApprovalMode::Suggestion;
        }
        self.run_agent_loop(output_mode, report).await
    }

    fn extend_history_for_next_turn<F>(
        &mut self,
        tool_results: Vec<Message>,
        report: &mut F,
        tool_rounds: usize,
    )
    where
        F: FnMut(AgentEvent) + Send,
    {
        self.history.extend(tool_results);
        report(AgentEvent::Status(
            "Tool results recorded. Advancing to the next agent step.".to_string(),
        ));
        self.history.push(
            self.runtime_continuation_message(RuntimeContinuationPhase::ToolResultsAvailable, tool_rounds),
        );
    }

    fn visible_tool_schemas(&self) -> Vec<Value> {
        self.tool_manager
            .get_schemas_filtered(|name| self.is_tool_allowed_in_current_mode(name))
    }

    fn current_compact_budget(&self) -> Option<ContextBudget> {
        let tools = self.visible_tool_schemas();
        self.llm_backend.context_budget(&self.history, &tools)
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

    fn capture_plan_from_text(&mut self, text: &str) -> bool {
        let Some((steps, explanation)) = parse_plan_block(text) else {
            self.pending_user_input = parse_request_user_input_block(text);
            return false;
        };
        if !steps.is_empty() {
            self.current_plan = steps;
        }
        self.plan_explanation = explanation;
        self.pending_user_input = parse_request_user_input_block(text);
        true
    }

    fn should_continue_plan_without_tools(
        &self,
        plan_updated: bool,
        _assistant_text: &str,
        tool_rounds: usize,
        plan_continuations: usize,
    ) -> bool {
        let shallow_initial_plan = plan_updated && tool_rounds == 0 && self.current_plan.len() <= 1;
        let still_missing_inspection_evidence =
            tool_rounds > 0
                && self.inspection_progress.has_any_evidence()
                && !self.inspection_progress.has_minimum_review_evidence();
        matches!(self.execution_mode, AgentExecutionMode::Plan)
            && (shallow_initial_plan || still_missing_inspection_evidence)
            && plan_continuations < MAX_PLAN_CONTINUATIONS_PER_TURN
            && self.pending_user_input.is_none()
            && !self.current_plan.is_empty()
    }

    fn should_continue_execute_without_tools(
        &self,
        assistant_text: &str,
        _tool_rounds: usize,
        execute_continuations: usize,
    ) -> bool {
        let inspection_intent =
            self.inspection_progress.has_any_evidence() || text_mentions_local_file_targets(assistant_text);
        matches!(self.execution_mode, AgentExecutionMode::Execute)
            && inspection_intent
            && execute_continuations < MAX_EXECUTE_CONTINUATIONS_PER_TURN
            && self.pending_user_input.is_none()
            && self.pending_approval.is_none()
            && !self.inspection_progress.has_minimum_review_evidence()
    }

    fn ensure_active_plan_step(&mut self) {
        if !matches!(self.execution_mode, AgentExecutionMode::Execute) || self.current_plan.is_empty() {
            return;
        }
        if self
            .current_plan
            .iter()
            .any(|step| matches!(step.status, PlanStepStatus::InProgress))
        {
            return;
        }
        if let Some(step) = self
            .current_plan
            .iter_mut()
            .find(|step| matches!(step.status, PlanStepStatus::Pending))
        {
            step.status = PlanStepStatus::InProgress;
        }
    }

    fn advance_plan_step(&mut self) {
        if !matches!(self.execution_mode, AgentExecutionMode::Execute) || self.current_plan.is_empty() {
            return;
        }
        if let Some(step) = self
            .current_plan
            .iter_mut()
            .find(|step| matches!(step.status, PlanStepStatus::InProgress))
        {
            step.status = PlanStepStatus::Completed;
        }
        if let Some(step) = self
            .current_plan
            .iter_mut()
            .find(|step| matches!(step.status, PlanStepStatus::Pending))
        {
            step.status = PlanStepStatus::InProgress;
        }
    }

    fn complete_remaining_plan_steps(&mut self) {
        if !matches!(self.execution_mode, AgentExecutionMode::Execute) || self.current_plan.is_empty() {
            return;
        }
        for step in &mut self.current_plan {
            if !matches!(step.status, PlanStepStatus::Completed) {
                step.status = PlanStepStatus::Completed;
            }
        }
    }

    fn runtime_continuation_message(
        &self,
        phase: RuntimeContinuationPhase,
        tool_rounds: usize,
    ) -> Message {
        let pending_interaction = if self.pending_approval.is_some() {
            "approval"
        } else if self.pending_user_input.is_some() {
            "request_user_input"
        } else {
            "none"
        };
        let payload = RuntimeContinuation {
            phase: phase.label(),
            mode: self.execution_mode_label(),
            tool_rounds,
            inspection: RuntimeInspectionSnapshot {
                list_calls: self.inspection_progress.list_calls,
                source_reads: self.inspection_progress.source_reads,
                config_reads: self.inspection_progress.config_reads,
                instruction_reads: self.inspection_progress.instruction_reads,
                has_minimum_review_evidence: self.inspection_progress.has_minimum_review_evidence(),
            },
            plan: RuntimePlanSnapshot {
                total_steps: self.current_plan.len(),
                pending_steps: self
                    .current_plan
                    .iter()
                    .filter(|step| matches!(step.status, PlanStepStatus::Pending))
                    .count(),
                in_progress_steps: self
                    .current_plan
                    .iter()
                    .filter(|step| matches!(step.status, PlanStepStatus::InProgress))
                    .count(),
                completed_steps: self
                    .current_plan
                    .iter()
                    .filter(|step| matches!(step.status, PlanStepStatus::Completed))
                    .count(),
            },
            pending_interaction,
            instructions: phase.instructions(),
        };
        let payload = serde_json::to_string_pretty(&payload)
            .unwrap_or_else(|_| "{\"phase\":\"tool_results_available\"}".to_string());
        Message {
            role: "user".to_string(),
            content: json!([{"type": "text", "text": format!("<agent_runtime>\n{payload}\n</agent_runtime>")}]),
        }
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

impl RuntimeContinuationPhase {
    fn label(self) -> &'static str {
        match self {
            Self::ToolResultsAvailable => "tool_results_available",
            Self::PlanContinuationRequired => "plan_continuation_required",
            Self::ExecutionContinuationRequired => "execution_continuation_required",
        }
    }

    fn instructions(self) -> Vec<&'static str> {
        match self {
            Self::ToolResultsAvailable => vec![
                "Continue the same task immediately.",
                "Review the tool results already present in the conversation.",
                "Either call the next tool directly, or provide the final answer.",
                "Do not ask the user to continue.",
                "Do not repeat tool results verbatim.",
            ],
            Self::PlanContinuationRequired => vec![
                "Continue planning immediately.",
                "Use read-only tools to inspect the repository before stopping.",
                "Expand the plan into multiple concrete steps grounded in the inspected code.",
                "Only stop planning when you have either gathered enough code context or need structured user input.",
                "Do not ask the user to continue.",
            ],
            Self::ExecutionContinuationRequired => vec![
                "Continue the same task immediately.",
                "Keep gathering the next relevant code context now.",
                "If you mentioned a next file or next inspection step, call the tool for it directly.",
                "Only stop when you can provide concrete, evidence-based suggestions or when structured user input is required.",
                "Do not ask the user to continue.",
            ],
        }
    }
}

fn text_mentions_local_file_targets(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [
        "cargo.toml",
        "cargo.lock",
        "agents.md",
        "readme.md",
        "src/",
        "src\\",
        ".rs",
        ".toml",
        ".md",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
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

fn estimate_history_tokens(history: &[Message]) -> Result<usize> {
    let bpe = tiktoken_rs::cl100k_base()?;
    Ok(history
        .iter()
        .map(|message| bpe.encode_with_special_tokens(&message.content.to_string()).len())
        .sum())
}

#[cfg(test)]
mod tests {
    use super::{parse_plan_block, parse_request_user_input_block, Agent, AgentExecutionMode, AnthropicResponse, ContentBlock, Message, PendingUserInput, PlanStep, PlanStepStatus, RuntimeContinuationPhase, TokenUsage};
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
        let continuation = agent.runtime_continuation_message(RuntimeContinuationPhase::ToolResultsAvailable, 1);
        assert!(second_round.iter().any(|message| message.content == continuation.content));
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
            .any(|message| message.content.to_string().contains("\"phase\": \"tool_results_available\"")));
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
            .query_with_mode("review-current-project".to_string(), super::AgentOutputMode::Silent)
            .await
            .expect("query should succeed");

        let observed_tools = backend.observed_tools();
        assert_eq!(observed_tools.len(), 1);
        assert_eq!(observed_tools[0], vec!["stub_tool".to_string()]);
    }

    #[tokio::test]
    async fn continues_plan_mode_after_shallow_initial_plan() {
        let backend = Arc::new(SequencedBackend::new(vec![
            AnthropicResponse {
                content: vec![ContentBlock::Text {
                    text: "<plan>\n- [pending] Inspect the repository structure\n</plan>\nStart with the top-level layout.".to_string(),
                }],
                stop_reason: Some("end_turn".to_string()),
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
        agent.set_execution_mode(AgentExecutionMode::Plan);

        agent
            .query_with_mode("inspect".to_string(), super::AgentOutputMode::Silent)
            .await
            .expect("query should succeed");

        let observed = backend.observed_messages.lock().expect("lock");
        assert_eq!(observed.len(), 2);
        assert!(agent
            .history
            .iter()
            .any(|message| message.content.to_string().contains("plan_continuation_required")));
    }

    #[tokio::test]
    async fn continues_plan_mode_after_exploration_if_assistant_still_signals_more_work() {
        let backend = Arc::new(SequencedBackend::new(vec![
            AnthropicResponse {
                content: vec![
                    ContentBlock::Text {
                        text: "<plan>\n- [pending] Inspect the repository structure\n</plan>\nStart with the top-level layout.".to_string(),
                    },
                    ContentBlock::ToolUse {
                        id: "tool-1".to_string(),
                        name: "stub_tool".to_string(),
                        input: json!({}),
                    },
                ],
                stop_reason: Some("tool_use".to_string()),
                usage: Some(TokenUsage::default()),
            },
            AnthropicResponse {
                content: vec![ContentBlock::Text {
                    text: "I have examined the overall structure. To provide detailed feedback, I will inspect the more complex components next.".to_string(),
                }],
                stop_reason: Some("end_turn".to_string()),
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
        agent.set_execution_mode(AgentExecutionMode::Plan);

        agent
            .query_with_mode("inspect".to_string(), super::AgentOutputMode::Silent)
            .await
            .expect("query should succeed");

        let observed = backend.observed_messages.lock().expect("lock");
        assert_eq!(observed.len(), 2);
    }

    #[tokio::test]
    async fn last_query_plan_updated_tracks_only_the_final_planning_turn() {
        let backend = Arc::new(SequencedBackend::new(vec![
            AnthropicResponse {
                content: vec![ContentBlock::Text {
                    text: "<plan>\n- [pending] Inspect the repository structure\n</plan>\nStart with the top-level layout.".to_string(),
                }],
                stop_reason: Some("end_turn".to_string()),
                usage: Some(TokenUsage::default()),
            },
            AnthropicResponse {
                content: vec![ContentBlock::Text {
                    text: "I still need to inspect the runtime and rendering paths before finalizing the plan.".to_string(),
                }],
                stop_reason: Some("end_turn".to_string()),
                usage: Some(TokenUsage::default()),
            },
        ]));

        let mut tool_manager = ToolManager::new();
        tool_manager.register(Box::new(StubTool));
        let mut agent = Agent::new(
            tool_manager,
            backend,
            Arc::new(VectorDB::new("data/lancedb")),
            Arc::new(SessionManager::new().expect("session manager")),
            Arc::new(WorkspaceMemory::new().expect("workspace memory")),
        );
        agent.set_execution_mode(AgentExecutionMode::Plan);

        agent
            .query_with_mode("inspect".to_string(), super::AgentOutputMode::Silent)
            .await
            .expect("query should succeed");

        assert!(!agent.last_query_produced_plan());
        assert_eq!(agent.current_plan.len(), 1);
    }

    #[tokio::test]
    async fn continues_execute_mode_after_exploration_if_assistant_still_signals_more_work() {
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
                    text: "I have checked the top-level structure. Next, I will inspect src/main.rs to understand the bootstrap flow.".to_string(),
                }],
                stop_reason: Some("end_turn".to_string()),
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
        agent.set_execution_mode(AgentExecutionMode::Execute);

        agent
            .query_with_mode("inspect".to_string(), super::AgentOutputMode::Silent)
            .await
            .expect("query should succeed");

        let observed = backend.observed_messages.lock().expect("lock");
        assert_eq!(observed.len(), 3);
        assert!(agent
            .history
            .iter()
            .any(|message| message.content.to_string().contains("execution_continuation_required")));
    }

    #[tokio::test]
    async fn continues_execute_mode_when_assistant_only_plans_followup_file_reads() {
        let backend = Arc::new(SequencedBackend::new(vec![
            AnthropicResponse {
                content: vec![ContentBlock::Text {
                    text: "I have checked the repository layout. Next, I will read Cargo.toml, AGENTS.md, and src/main.rs to understand the bootstrap and architecture.".to_string(),
                }],
                stop_reason: Some("end_turn".to_string()),
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

        let mut agent = Agent::new(
            ToolManager::new(),
            backend.clone(),
            Arc::new(VectorDB::new("data/lancedb")),
            Arc::new(SessionManager::new().expect("session manager")),
            Arc::new(WorkspaceMemory::new().expect("workspace memory")),
        );
        agent.set_execution_mode(AgentExecutionMode::Execute);

        agent
            .query_with_mode("inspect".to_string(), super::AgentOutputMode::Silent)
            .await
            .expect("query should succeed");

        let observed = backend.observed_messages.lock().expect("lock");
        assert_eq!(observed.len(), 2);
        assert!(agent
            .history
            .iter()
            .any(|message| message.content.to_string().contains("execution_continuation_required")));
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

    #[test]
    fn advances_plan_steps_during_execute_mode() {
        let mut agent = Agent::new(
            ToolManager::new(),
            Arc::new(SequencedBackend::new(Vec::new())),
            Arc::new(VectorDB::new("data/lancedb")),
            Arc::new(SessionManager::new().expect("session manager")),
            Arc::new(WorkspaceMemory::new().expect("workspace memory")),
        );
        agent.set_execution_mode(AgentExecutionMode::Execute);
        agent.current_plan = vec![
            PlanStep {
                step: "Inspect code".to_string(),
                status: PlanStepStatus::Pending,
            },
            PlanStep {
                step: "Apply changes".to_string(),
                status: PlanStepStatus::Pending,
            },
        ];

        agent.ensure_active_plan_step();
        assert_eq!(agent.current_plan[0].status, PlanStepStatus::InProgress);
        assert_eq!(agent.current_plan[1].status, PlanStepStatus::Pending);

        agent.advance_plan_step();
        assert_eq!(agent.current_plan[0].status, PlanStepStatus::Completed);
        assert_eq!(agent.current_plan[1].status, PlanStepStatus::InProgress);
    }

    #[test]
    fn completes_remaining_plan_steps_on_finish() {
        let mut agent = Agent::new(
            ToolManager::new(),
            Arc::new(SequencedBackend::new(Vec::new())),
            Arc::new(VectorDB::new("data/lancedb")),
            Arc::new(SessionManager::new().expect("session manager")),
            Arc::new(WorkspaceMemory::new().expect("workspace memory")),
        );
        agent.set_execution_mode(AgentExecutionMode::Execute);
        agent.current_plan = vec![
            PlanStep {
                step: "Inspect code".to_string(),
                status: PlanStepStatus::Completed,
            },
            PlanStep {
                step: "Apply changes".to_string(),
                status: PlanStepStatus::InProgress,
            },
            PlanStep {
                step: "Summarize".to_string(),
                status: PlanStepStatus::Pending,
            },
        ];

        agent.complete_remaining_plan_steps();

        assert!(agent
            .current_plan
            .iter()
            .all(|step| step.status == PlanStepStatus::Completed));
    }

    #[tokio::test]
    async fn manual_compact_replaces_older_history_with_summary() {
        let backend = Arc::new(SequencedBackend::new(Vec::new()));
        let mut agent = Agent::new(
            ToolManager::new(),
            backend,
            Arc::new(VectorDB::new("data/lancedb")),
            Arc::new(SessionManager::new().expect("session manager")),
            Arc::new(WorkspaceMemory::new().expect("workspace memory")),
        );
        agent.history = vec![
            Message {
                role: "user".to_string(),
                content: json!("inspect the repo"),
            },
            Message {
                role: "assistant".to_string(),
                content: json!("I checked Cargo.toml and src/main.rs"),
            },
        ];

        let compacted = agent
            .compact_now_with_reporter(|_| {})
            .await
            .expect("compact should succeed");

        assert!(compacted);
        assert_eq!(agent.compact_state.compaction_count, 1);
        assert!(agent.history[0]
            .content
            .to_string()
            .contains("SUMMARY OF PREVIOUS CONVERSATION"));
    }
}
