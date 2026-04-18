use super::*;
use anyhow::anyhow;
use crate::tools::bash::BashCommandInput;

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
    pub request: BashCommandInput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletedInteraction {
    pub title: String,
    pub summary: String,
}

#[derive(Debug, Clone, Default)]
pub(super) struct InspectionProgress {
    pub(super) list_calls: usize,
    pub(super) source_reads: usize,
    pub(super) config_reads: usize,
    pub(super) instruction_reads: usize,
}

#[derive(Clone, Copy)]
pub(super) enum RuntimeContinuationPhase {
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
    pub(super) fn record_tool(&mut self, name: &str, input: &Value) {
        match name {
            "list_files" => {
                self.list_calls += 1;
            }
            "read_file" => {
                let path = input
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .replace('\\', "/");
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

    pub(super) fn has_minimum_review_evidence(&self) -> bool {
        self.source_reads >= 2
            || (self.source_reads >= 1
                && self.list_calls >= 1
                && (self.config_reads >= 1 || self.instruction_reads >= 1))
    }

    pub(super) fn has_any_evidence(&self) -> bool {
        self.list_calls > 0
            || self.source_reads > 0
            || self.config_reads > 0
            || self.instruction_reads > 0
    }
}

impl Agent {
    pub fn last_query_produced_plan(&self) -> bool {
        self.last_query_plan_updated
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
        F: FnMut(AgentEvent) + Send,
    {
        let pending = self
            .pending_approval
            .clone()
            .ok_or_else(|| anyhow!("No pending approval to answer"))?;

        self.pending_approval = None;
        self.pending_user_input = None;
        self.completed_approval = None;

        match selection {
            BashApprovalMode::Once => {
                self.completed_approval = Some(CompletedInteraction {
                    title: "Bash approval".to_string(),
                    summary: format!("Approved once for command: {}", pending.request.summary()),
                });
                self.execute_pending_bash(pending, false, output_mode, &mut report)
                    .await?;
            }
            BashApprovalMode::Always => {
                self.bash_approval_mode = BashApprovalMode::Always;
                self.completed_approval = Some(CompletedInteraction {
                    title: "Bash approval".to_string(),
                    summary: format!("Approved for session: {}", pending.request.summary()),
                });
                self.execute_pending_bash(pending, true, output_mode, &mut report)
                    .await?;
            }
            BashApprovalMode::Suggestion => {
                self.completed_approval = Some(CompletedInteraction {
                    title: "Bash approval".to_string(),
                    summary: format!("Kept as suggestion only: {}", pending.request.summary()),
                });
                let error_text = "Bash command was not approved. Continue without shell execution and find a safer path.".to_string();
                report(AgentEvent::ToolResult {
                    name: "bash".to_string(),
                    content: error_text.clone(),
                    is_error: true,
                });
                self.history.push(tool_result_message(&pending.tool_use_id, error_text, true));
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
        let input = pending.request.to_value();
        report(AgentEvent::ToolUse {
            name: "bash".to_string(),
            input: input.clone(),
        });
        let tool = self
            .tool_manager
            .get_tool("bash")
            .ok_or_else(|| anyhow!("bash tool is unavailable"))?;
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

    pub(super) fn extend_history_for_next_turn<F>(
        &mut self,
        tool_results: Vec<Message>,
        report: &mut F,
        tool_rounds: usize,
    ) where
        F: FnMut(AgentEvent) + Send,
    {
        self.history.extend(tool_results);
        report(AgentEvent::Status(
            "Tool results recorded. Advancing to the next agent step.".to_string(),
        ));
        self.history.push(self.runtime_continuation_message(
            RuntimeContinuationPhase::ToolResultsAvailable,
            tool_rounds,
        ));
    }

    pub(super) fn visible_tool_schemas(&self) -> Vec<Value> {
        self.tool_manager
            .get_schemas_filtered(|name| self.is_tool_allowed_in_current_mode(name))
    }

    pub(super) fn is_tool_allowed_in_current_mode(&self, name: &str) -> bool {
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

    pub(super) fn capture_plan_from_text(&mut self, text: &str) -> bool {
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

    pub(super) fn should_continue_plan_without_tools(
        &self,
        plan_updated: bool,
        _assistant_text: &str,
        tool_rounds: usize,
        plan_continuations: usize,
    ) -> bool {
        let shallow_initial_plan = plan_updated && tool_rounds == 0 && self.current_plan.len() <= 1;
        let still_missing_inspection_evidence = tool_rounds > 0
            && self.inspection_progress.has_any_evidence()
            && !self.inspection_progress.has_minimum_review_evidence();
        matches!(self.execution_mode, AgentExecutionMode::Plan)
            && (shallow_initial_plan || still_missing_inspection_evidence)
            && plan_continuations < MAX_PLAN_CONTINUATIONS_PER_TURN
            && self.pending_user_input.is_none()
            && !self.current_plan.is_empty()
    }

    pub(super) fn should_continue_execute_without_tools(
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

    pub(super) fn ensure_active_plan_step(&mut self) {
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

    pub(super) fn advance_plan_step(&mut self) {
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

    pub(super) fn complete_remaining_plan_steps(&mut self) {
        if !matches!(self.execution_mode, AgentExecutionMode::Execute) || self.current_plan.is_empty() {
            return;
        }
        for step in &mut self.current_plan {
            if !matches!(step.status, PlanStepStatus::Completed) {
                step.status = PlanStepStatus::Completed;
            }
        }
    }

    pub(super) fn runtime_continuation_message(
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

pub(super) fn parse_plan_block(text: &str) -> Option<(Vec<PlanStep>, Option<String>)> {
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
    Some((steps, (!explanation.is_empty()).then(|| explanation.to_string())))
}

pub(super) fn parse_request_user_input_block(text: &str) -> Option<PendingUserInput> {
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

pub(super) fn tool_result_message(tool_use_id: &str, content: String, is_error: bool) -> Message {
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
