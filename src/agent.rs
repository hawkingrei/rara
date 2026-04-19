mod compact;
mod planning;
mod prompting;
#[cfg(test)]
mod tests;

use crate::llm::LlmBackend;
use crate::prompt::{self, PromptMode, PromptRuntimeConfig};
use crate::session::SessionManager;
use crate::tool::ToolManager;
use crate::tool_result::{
    default_tool_result_store_dir, repair_tool_result_history, ToolResultStore,
};
use crate::tools::bash::BashCommandInput;
use crate::vectordb::{MemoryMetadata, VectorDB};
use crate::workspace::WorkspaceMemory;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

pub use self::compact::{latest_compact_boundary_metadata, CompactBoundaryMetadata, CompactState};
use self::planning::{
    tool_result_message, InspectionProgress, PlanningOutcomeContract, RuntimeContinuationPhase,
};
pub use self::planning::{
    CompletedInteraction, PendingApproval, PendingUserInput, PlanStep, PlanStepStatus,
};

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

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
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
    ToolUse {
        name: String,
        input: Value,
    },
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
    continue_inspection: bool,
    had_text_response: bool,
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
    inspection_progress: InspectionProgress,
    last_query_plan_updated: bool,
    prompt_config: PromptRuntimeConfig,
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
            inspection_progress: InspectionProgress::default(),
            last_query_plan_updated: false,
            prompt_config: PromptRuntimeConfig::default(),
        }
    }

    pub async fn query(&mut self, prompt: String) -> Result<()> {
        self.query_with_mode(prompt, AgentOutputMode::Terminal)
            .await
    }

    pub async fn query_with_mode(
        &mut self,
        prompt: String,
        output_mode: AgentOutputMode,
    ) -> Result<()> {
        self.query_with_mode_and_events(prompt, output_mode, |_| {})
            .await
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
        self.inspection_progress = InspectionProgress::default();
        self.last_query_plan_updated = false;
        self.compact_if_needed_with_reporter(&mut report).await?;
        let repaired_history = repair_tool_result_history(&self.history);
        if repaired_history != self.history {
            self.replace_history(repaired_history);
        }
        self.clear_completed_interactions();

        self.push_history_message(Message {
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

        self.session_manager
            .save_session(&self.session_id, &self.history)?;
        let turn_text = format!(
            "User: {}\nAgent Response: {:?}",
            prompt,
            self.history.last().unwrap().content
        );
        if let Ok(vector) = self.llm_backend.embed(&turn_text).await {
            let _ = self
                .vdb
                .upsert_turn(
                    "conversations",
                    MemoryMetadata {
                        session_id: self.session_id.clone(),
                        turn_index: turn_start_idx as u32,
                        text: turn_text,
                    },
                    vector,
                )
                .await;
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
        let mut messages = self
            .history
            .iter()
            .filter(|message| !is_compact_boundary_message(message))
            .cloned()
            .collect::<Vec<_>>();
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
        let mut continue_inspection = false;
        let mut had_text_response = false;
        let mut sanitized_content = Vec::new();
        for block in &response.content {
            match block {
                ContentBlock::Text { text } => {
                    let (clean_text, block_requests_continue) =
                        planning::strip_continue_inspection_control(text);
                    continue_inspection |= block_requests_continue;
                    if !clean_text.trim().is_empty() {
                        had_text_response = true;
                        sanitized_content.push(ContentBlock::Text {
                            text: clean_text.clone(),
                        });
                        if !streamed_any_delta {
                            report(AgentEvent::AssistantText(clean_text.clone()));
                        }
                        if matches!(self.execution_mode, AgentExecutionMode::Plan) {
                            plan_updated |= self.capture_plan_from_text(&clean_text);
                        }
                        if matches!(output_mode, AgentOutputMode::Terminal) {
                            println!("Agent: {}", clean_text);
                        }
                    }
                }
                ContentBlock::ToolUse { id, name, input } => {
                    sanitized_content.push(ContentBlock::ToolUse {
                        id: id.clone(),
                        name: name.clone(),
                        input: input.clone(),
                    });
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
                content: serde_json::to_value(&sanitized_content)?,
            },
            tool_calls,
            plan_updated,
            continue_inspection,
            had_text_response,
        })
    }

    async fn run_agent_loop<F>(
        &mut self,
        output_mode: AgentOutputMode,
        report: &mut F,
    ) -> Result<()>
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
            let turn_output = self.run_model_turn(output_mode, report).await?;
            self.last_query_plan_updated = turn_output.plan_updated;
            self.push_history_message(turn_output.assistant_message);

            if turn_output.tool_calls.is_empty() {
                if self.should_continue_plan_without_tools(
                    turn_output.plan_updated,
                    turn_output.continue_inspection,
                    turn_output.had_text_response,
                    *tool_rounds,
                    *plan_continuations,
                ) {
                    *plan_continuations += 1;
                    let phase = if matches!(
                        self.planning_outcome_contract(
                            turn_output.plan_updated,
                            turn_output.continue_inspection,
                            turn_output.had_text_response,
                        ),
                        PlanningOutcomeContract::StructuredOutcomeRequired
                    ) {
                        report(AgentEvent::Status(
                            "Planning mode needs a concrete plan, question, or explicit continue signal. Continuing in read-only mode."
                                .to_string(),
                        ));
                        RuntimeContinuationPhase::PlanStructuredOutcomeRequired
                    } else {
                        report(AgentEvent::Status(
                            "Plan needs more repository inspection. Continuing in read-only mode."
                                .to_string(),
                        ));
                        RuntimeContinuationPhase::PlanContinuationRequired
                    };
                    self.push_history_message(self.runtime_continuation_message(
                        phase,
                        *tool_rounds,
                    ));
                    continue;
                }
                if self.should_continue_execute_without_tools(
                    *tool_rounds,
                    *execute_continuations,
                    turn_output.continue_inspection,
                ) {
                    *execute_continuations += 1;
                    report(AgentEvent::Status(
                        "Repository review needs more code inspection. Continuing the same turn."
                            .to_string(),
                    ));
                    self.push_history_message(self.runtime_continuation_message(
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
            if tool_call.name == "bash"
                && matches!(self.bash_approval_mode, BashApprovalMode::Suggestion)
            {
                let request =
                    BashCommandInput::from_value(tool_call.input.clone()).unwrap_or_else(|_| {
                        BashCommandInput {
                            command: tool_call
                                .input
                                .get("command")
                                .and_then(Value::as_str)
                                .map(str::to_string),
                            program: None,
                            args: Vec::new(),
                            cwd: None,
                            env: Default::default(),
                            allow_net: tool_call
                                .input
                                .get("allow_net")
                                .and_then(Value::as_bool)
                                .unwrap_or(false),
                        }
                    });
                let summary = request.summary();
                self.pending_approval = Some(PendingApproval {
                    tool_use_id: tool_call.id.clone(),
                    request: request.clone(),
                });
                self.pending_user_input = Some(PendingUserInput {
                    question: "Bash command needs approval. What should RARA do?".to_string(),
                    options: vec![
                        (
                            "Run once".to_string(),
                            "Execute this command now and then return to suggestion mode."
                                .to_string(),
                        ),
                        (
                            "Always allow bash".to_string(),
                            "Execute now and keep bash approval open for later commands."
                                .to_string(),
                        ),
                        (
                            "Suggestion only".to_string(),
                            "Do not run the command automatically. Continue with a safer path."
                                .to_string(),
                        ),
                    ],
                    note: Some(format!("command: {}", summary)),
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
                tool_results.push(tool_result_message(&tool_call.id, error_text, true));
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
                        tool_results.push(tool_result_message(&tool_call.id, result_text, false));
                    }
                    Err(e) => {
                        let error_text = format!("Error: {}", e);
                        report(AgentEvent::ToolResult {
                            name: tool_call.name.clone(),
                            content: error_text.clone(),
                            is_error: true,
                        });
                        tool_results.push(tool_result_message(&tool_call.id, error_text, true));
                    }
                }
            }
        }
        Ok(tool_results)
    }
}

fn is_compact_boundary_message(message: &Message) -> bool {
    message.role == "system"
        && message
            .content
            .get("type")
            .and_then(Value::as_str)
            == Some("compact_boundary")
}
