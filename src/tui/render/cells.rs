use std::path::Path;

use ratatui::{style::Color, text::Line};

use crate::tui::interaction_text::{
    pending_interaction_detail_text, pending_interaction_shortcut_text,
};
use crate::tui::plan_display::should_show_updated_plan;
use crate::tui::state::{
    contains_structured_planning_output, RuntimePhase, TranscriptEntry, TuiApp,
};

#[path = "cells_components.rs"]
mod components;

pub(crate) use self::components::StartupCardCell;
use self::components::{
    CommittedInteractionCell, ExploredCell, ExploringCell, MessageCell, PendingInteractionCell,
    PlanModeCell, PlanSummaryCell, PlanningCell, PlanningSuggestionCell, RanCell, RespondingCell,
    RunningCell, UserCell, planning_suggestion_text,
};
use super::{
    compact_summary_lines, compact_summary_text, current_turn_exploration_summary,
    current_turn_exploration_summary_from_entries,
    current_turn_tool_summary,
    history_pipeline::{narrative_entries, ordered_completion_entries},
    wrapped_history_line_count,
};

pub(crate) trait HistoryCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>>;

    fn desired_height(&self, width: u16) -> u16 {
        wrapped_history_line_count(self.display_lines(width).as_slice(), width)
    }
}

pub(crate) trait ActiveCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>>;
}

fn trim_trailing_empty_lines(lines: &mut Vec<Line<'static>>) {
    while matches!(lines.last(), Some(line) if line.spans.iter().all(|span| span.content == "")) {
        lines.pop();
    }
}

#[derive(Clone, Copy)]
pub(super) enum InteractionCompletionKind {
    ShellApprovalCompleted,
    PlanDecision,
    QuestionAnswered,
    PlanningQuestionAnswered,
    ExplorationQuestionAnswered,
    SubAgentQuestionAnswered,
}

impl InteractionCompletionKind {
    fn from_role(role: &str) -> Option<Self> {
        match role {
            "Shell Approval Completed" => Some(Self::ShellApprovalCompleted),
            "Plan Decision" => Some(Self::PlanDecision),
            "Question Answered" => Some(Self::QuestionAnswered),
            "Planning Question Answered" => Some(Self::PlanningQuestionAnswered),
            "Exploration Question Answered" => Some(Self::ExplorationQuestionAnswered),
            "Sub-agent Question Answered" => Some(Self::SubAgentQuestionAnswered),
            _ => None,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::ShellApprovalCompleted => "Shell Approval Completed",
            Self::PlanDecision => "Plan Decision",
            Self::QuestionAnswered => "Question Answered",
            Self::PlanningQuestionAnswered => "Planning Question Answered",
            Self::ExplorationQuestionAnswered => "Exploration Question Answered",
            Self::SubAgentQuestionAnswered => "Sub-agent Question Answered",
        }
    }

    fn color(self) -> Color {
        match self {
            Self::PlanDecision => Color::Cyan,
            Self::ShellApprovalCompleted
            | Self::QuestionAnswered
            | Self::PlanningQuestionAnswered
            | Self::ExplorationQuestionAnswered
            | Self::SubAgentQuestionAnswered => Color::LightGreen,
        }
    }
}

fn completion_role_color(role: &str) -> Option<Color> {
    InteractionCompletionKind::from_role(role).map(|kind| kind.color())
}

fn completion_role_kind(role: &str) -> Option<InteractionCompletionKind> {
    InteractionCompletionKind::from_role(role)
}

pub(crate) struct CommittedTurnCell<'a> {
    entries: &'a [TranscriptEntry],
    cwd: Option<&'a Path>,
}

impl<'a> CommittedTurnCell<'a> {
    pub(crate) fn new(entries: &'a [TranscriptEntry], cwd: Option<&'a Path>) -> Self {
        Self { entries, cwd }
    }
}

impl HistoryCell for CommittedTurnCell<'_> {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut cells: Vec<Box<dyn HistoryCell + '_>> = Vec::new();
        if let Some(user) = self.entries.iter().find(|entry| entry.role == "You") {
            cells.push(Box::new(UserCell::new(user.message.clone())));
        }

        let entry_refs = self.entries.iter().collect::<Vec<_>>();
        let explicit_exploration = self
            .entries
            .iter()
            .find(|entry| entry.role == "Exploring")
            .map(|entry| entry.message.clone());
        let explicit_planning = self
            .entries
            .iter()
            .find(|entry| entry.role == "Planning")
            .map(|entry| entry.message.clone());
        let explicit_running = self
            .entries
            .iter()
            .find(|entry| entry.role == "Running")
            .map(|entry| entry.message.clone());
        let has_tool_activity = entry_refs
            .iter()
            .any(|entry| matches!(entry.role.as_str(), "Tool" | "Tool Result" | "Tool Error" | "Tool Progress"));
        if let Some(summary) = explicit_exploration
            .map(|summary| compact_summary_text(&summary, 4, "more exploration step(s)"))
            .or_else(|| {
                current_turn_exploration_summary_from_entries(entry_refs.as_slice(), false, None)
            })
        {
            cells.push(Box::new(ExploredCell::new(summary)));
        }

        if let Some(summary) = explicit_planning {
            cells.push(Box::new(PlanningCell::new(summary, false)));
        }

        if let Some(summary) = explicit_running
            .or_else(|| current_turn_tool_summary(entry_refs.as_slice(), false, None))
        {
            cells.push(Box::new(RanCell::new(summary)));
        }

        let completion_entries = ordered_completion_entries(self.entries);
        let narrative_entries = narrative_entries(
            self.entries,
            has_tool_activity,
            is_renderable_system_message,
        );

        for entry in completion_entries {
            let kind = match entry.kind {
                super::history_pipeline::CommittedCompletionKind::ShellApprovalCompleted => {
                    InteractionCompletionKind::ShellApprovalCompleted
                }
                super::history_pipeline::CommittedCompletionKind::PlanDecision => {
                    InteractionCompletionKind::PlanDecision
                }
                super::history_pipeline::CommittedCompletionKind::PlanningQuestionAnswered => {
                    InteractionCompletionKind::PlanningQuestionAnswered
                }
                super::history_pipeline::CommittedCompletionKind::ExplorationQuestionAnswered => {
                    InteractionCompletionKind::ExplorationQuestionAnswered
                }
                super::history_pipeline::CommittedCompletionKind::SubAgentQuestionAnswered => {
                    InteractionCompletionKind::SubAgentQuestionAnswered
                }
                super::history_pipeline::CommittedCompletionKind::QuestionAnswered => {
                    InteractionCompletionKind::QuestionAnswered
                }
            };
            cells.push(Box::new(CommittedInteractionCell::new(kind, entry.message)));
        }

        for entry in narrative_entries {
            let max_lines = if entry.role == "Agent" { usize::MAX } else { 4 };
            cells.push(Box::new(MessageCell::new(
                &entry.role,
                &entry.message,
                max_lines,
                self.cwd,
            )));
        }

        let mut lines = Vec::new();
        for (idx, cell) in cells.into_iter().enumerate() {
            if idx > 0 {
                lines.push(Line::from(""));
            }
            lines.extend(cell.display_lines(width));
        }

        trim_trailing_empty_lines(&mut lines);
        lines
    }
}

pub(crate) struct ActiveTurnCell<'a> {
    app: &'a TuiApp,
    cwd: Option<&'a Path>,
}

impl<'a> ActiveTurnCell<'a> {
    pub(crate) fn new(app: &'a TuiApp, cwd: Option<&'a Path>) -> Self {
        Self { app, cwd }
    }
}

impl ActiveCell for ActiveTurnCell<'_> {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let current_turn = self.app.active_turn.entries.iter().collect::<Vec<_>>();
        let turn_live = self.app.is_busy()
            || matches!(
                self.app.runtime_phase,
                RuntimePhase::SendingPrompt
                    | RuntimePhase::ProcessingResponse
                    | RuntimePhase::RunningTool
            );
        if current_turn.is_empty() {
            if let Some(prompt) = self.app.pending_planning_suggestion.as_deref() {
                let cells: Vec<Box<dyn HistoryCell + '_>> = vec![
                    Box::new(UserCell::new(prompt)),
                    Box::new(PlanningSuggestionCell::new(
                        planning_suggestion_text(self.app),
                    )),
                ];
                let mut lines = Vec::new();
                for (idx, cell) in cells.into_iter().enumerate() {
                    if idx > 0 {
                        lines.push(Line::from(""));
                    }
                    lines.extend(cell.display_lines(width));
                }
                trim_trailing_empty_lines(&mut lines);
                return lines;
            }
            return Vec::new();
        }
        let has_tool_activity = current_turn
            .iter()
            .any(|entry| matches!(entry.role.as_str(), "Tool" | "Tool Result" | "Tool Error" | "Tool Progress"));
        let user_message = current_turn
            .iter()
            .find(|entry| entry.role == "You")
            .map(|entry| entry.message.as_str())
            .unwrap_or("");
        let latest_agent = current_turn
            .iter()
            .rev()
            .find(|entry| entry.role == "Agent")
            .map(|entry| entry.message.as_str());
        let streaming_agent_lines = self.app.agent_stream_lines();
        let has_agent_stream = self.app.has_agent_stream();
        let latest_system = current_turn
            .iter()
            .rev()
            .find(|entry| {
                entry.role == "System" && is_renderable_system_message(entry.message.as_str())
            })
            .map(|entry| entry.message.as_str());
        let latest_tool_result = current_turn
            .iter()
            .rev()
            .find(|entry| {
                matches!(
                    entry.role.as_str(),
                    "Tool Result" | "Tool Error" | "Tool Progress"
                )
            })
            .map(|entry| (entry.role.as_str(), entry.message.as_str()));
        let latest_completion = current_turn
            .iter()
            .rev()
            .find(|entry| completion_role_color(entry.role.as_str()).is_some());
        let mut cells: Vec<Box<dyn HistoryCell + '_>> = Vec::new();
        let has_live_exploration = !self.app.active_live.exploration_actions.is_empty()
            || !self.app.active_live.exploration_notes.is_empty();
        let has_live_planning = !self.app.active_live.planning_actions.is_empty()
            || !self.app.active_live.planning_notes.is_empty();
        let has_live_running = !self.app.active_live.running_actions.is_empty();

        if !user_message.is_empty() {
            cells.push(Box::new(UserCell::new(user_message)));
        }

        if self.app.agent_execution_mode_label() == "plan" && !self.app.has_pending_plan_approval()
        {
            cells.push(Box::new(PlanModeCell));
        }

        let explicit_exploration = current_turn
            .iter()
            .find(|entry| entry.role == "Exploring")
            .map(|entry| entry.message.clone());

        let exploration_summary = if has_live_exploration {
            let mut items = self
                .app
                .active_live
                .exploration_actions
                .iter()
                .cloned()
                .collect::<Vec<_>>();
            items.extend(
                self.app
                    .active_live
                    .exploration_notes
                    .iter()
                    .cloned(),
            );
            Some(compact_summary_lines(
                items.as_slice(),
                4,
                "more exploration step(s)",
            ))
        } else if !turn_live {
            None
        } else {
            explicit_exploration
                .map(|summary| compact_summary_text(&summary, 4, "more exploration step(s)"))
                .or_else(|| {
                    current_turn_exploration_summary(self.app, current_turn.as_slice(), turn_live)
                })
        };
        let has_exploration_summary = exploration_summary.is_some();
        let exploration_active = turn_live && has_exploration_summary;
        if let Some(summary) = exploration_summary {
            cells.push(Box::new(ExploringCell::new(summary, exploration_active)));
        }

        let explicit_planning = current_turn
            .iter()
            .find(|entry| entry.role == "Planning")
            .map(|entry| entry.message.clone());

        let planning_summary = if has_live_planning {
            let mut items = self
                .app
                .active_live
                .planning_actions
                .iter()
                .cloned()
                .collect::<Vec<_>>();
            items.extend(
                self.app
                    .active_live
                    .planning_notes
                    .iter()
                    .cloned(),
            );
            Some(compact_summary_lines(
                items.as_slice(),
                4,
                "more planning step(s)",
            ))
        } else {
            explicit_planning
                .map(|summary| compact_summary_text(&summary, 4, "more planning step(s)"))
        };
        let has_planning_summary = planning_summary.is_some();
        if let Some(summary) = planning_summary {
            cells.push(Box::new(PlanningCell::new(summary, turn_live)));
        }

        let explicit_running = current_turn
            .iter()
            .find(|entry| entry.role == "Running")
            .map(|entry| entry.message.clone());

        let running_summary = if has_live_running {
            let items = self
                .app
                .active_live
                .running_actions
                .iter()
                .cloned()
                .collect::<Vec<_>>();
            Some(compact_summary_lines(
                items.as_slice(),
                4,
                "more running step(s)",
            ))
        } else {
            explicit_running
                .map(|summary| compact_summary_text(&summary, 4, "more running step(s)"))
                .or_else(|| {
                current_turn_tool_summary(
                    current_turn.as_slice(),
                    turn_live,
                    self.app.runtime_phase_detail.as_deref(),
                )
            })
        };
        let has_running_summary = running_summary.is_some();
        let running_active = turn_live && has_running_summary;
        if let Some(summary) = running_summary {
            cells.push(Box::new(RunningCell::new(summary, running_active)));
        }

        if should_show_updated_plan(self.app) {
            cells.push(Box::new(PlanSummaryCell::new(
                self.app.snapshot.plan_steps.clone(),
                self.app.snapshot.plan_explanation.clone(),
            )));
        }

        if let Some(pending) = self.app.active_pending_interaction() {
            let mut request_lines = pending_interaction_detail_text(self.app, pending.kind)
                .lines()
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            request_lines.push(pending_interaction_shortcut_text(pending.kind).to_string());
            cells.push(Box::new(PendingInteractionCell::new(
                pending.kind,
                request_lines,
            )));
        }

        if self.app.pending_planning_suggestion.is_some() {
            cells.push(Box::new(PlanningSuggestionCell::new(
                planning_suggestion_text(self.app),
            )));
        }

        if let Some(entry) = latest_completion {
            if let Some(kind) = completion_role_kind(entry.role.as_str()) {
                cells.push(Box::new(CommittedInteractionCell::new(
                    kind,
                    entry.message.clone(),
                )));
            }
        }

        let suppress_intermediate_agent = turn_live
            && has_tool_activity
            && matches!(
                self.app.runtime_phase,
                RuntimePhase::RunningTool | RuntimePhase::SendingPrompt
            );
        let suppress_planning_chatter = matches!(
            self.app.agent_execution_mode,
            crate::agent::AgentExecutionMode::Plan
        ) && has_exploration_summary
            && latest_agent.is_some_and(|message| !contains_structured_planning_output(message))
            && self.app.snapshot.plan_steps.is_empty()
            && self.app.pending_request_input().is_none()
            && !self.app.has_pending_plan_approval();
        let suppress_structured_plan_response = !self.app.snapshot.plan_steps.is_empty()
            && latest_agent.is_some_and(contains_structured_planning_output);

        let responding_role = if turn_live { "Responding" } else { "Agent" };
        let prefer_responding_chrome = turn_live
            && matches!(
                self.app.runtime_phase,
                RuntimePhase::SendingPrompt | RuntimePhase::ProcessingResponse
            );

        if has_agent_stream
            && !suppress_intermediate_agent
            && !suppress_planning_chatter
            && !suppress_structured_plan_response
        {
            if let Some(stream_lines) = streaming_agent_lines {
                cells.push(Box::new(RespondingCell::from_stream(stream_lines)));
            } else if let Some(agent_message) = latest_agent {
                cells.push(Box::new(RespondingCell::from_message(
                    responding_role,
                    agent_message,
                    usize::MAX,
                    self.cwd,
                )));
            } else {
                cells.push(Box::new(RespondingCell::working(
                    self.app
                        .runtime_phase_detail
                        .as_deref()
                        .unwrap_or("streaming model output"),
                )));
            }
        } else if let Some(agent_message) = latest_agent.filter(|_| {
                !suppress_intermediate_agent
                    && !suppress_planning_chatter
                    && !suppress_structured_plan_response
            }) {
            cells.push(Box::new(RespondingCell::from_message(
                responding_role,
                agent_message,
                usize::MAX,
                self.cwd,
            )));
        } else if prefer_responding_chrome {
            cells.push(Box::new(RespondingCell::working(
                self.app
                    .runtime_phase_detail
                    .as_deref()
                    .unwrap_or("waiting for model output"),
            )));
        } else if let Some(system_message) = latest_system {
            cells.push(Box::new(RespondingCell::from_message(
                "System",
                system_message,
                14,
                self.cwd,
            )));
        } else if let Some((role, tool_result)) = latest_tool_result {
            cells.push(Box::new(RespondingCell::from_tool_result(
                role,
                tool_result,
                14,
            )));
        } else if turn_live
            && !has_exploration_summary
            && !has_planning_summary
            && !has_running_summary
            && self.app.pending_request_input().is_none()
            && !self.app.has_pending_plan_approval()
            && self.app.pending_command_approval().is_none()
            && self.app.pending_planning_suggestion.is_none()
            && self.app.snapshot.plan_steps.is_empty()
        {
            cells.push(Box::new(RespondingCell::working(
                self.app
                    .runtime_phase_detail
                    .as_deref()
                    .unwrap_or("waiting for the current turn to finish"),
            )));
        }

        let mut lines = Vec::new();
        for (idx, cell) in cells.into_iter().enumerate() {
            if idx > 0 {
                lines.push(Line::from(""));
            }
            lines.extend(cell.display_lines(width));
        }

        trim_trailing_empty_lines(&mut lines);
        lines
    }
}

fn is_renderable_system_message(message: &str) -> bool {
    let lower = message.trim().to_ascii_lowercase();
    lower.starts_with("query failed:")
        || lower.starts_with("compaction failed:")
        || lower.starts_with("compact failed:")
        || lower.starts_with("oauth failed:")
        || lower.starts_with("backend rebuild failed:")
        || lower.starts_with("open this url in a browser and enter the one-time code:")
        || lower.starts_with("starting codex browser login.")
        || lower.starts_with("error:")
}

#[cfg(test)]
#[path = "cells_tests.rs"]
mod tests;
