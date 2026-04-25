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
    planning_suggestion_text, CommittedInteractionCell, ExploredCell, ExploringCell, MessageCell,
    PendingInteractionCell, PlanModeCell, PlanSummaryCell, PlanningCell, PlanningSuggestionCell,
    RanCell, RespondingCell, RunningCell, UserCell,
};
use super::{
    compact_progress_summary_lines, compact_recent_first_summary_lines, compact_summary_lines,
    compact_summary_text, current_turn_exploration_summary,
    current_turn_exploration_summary_from_entries, current_turn_tool_summary,
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

enum OrderedActiveSegment<'a> {
    Exploration(Vec<String>),
    Agent(&'a str),
}

fn trim_trailing_empty_lines(lines: &mut Vec<Line<'static>>) {
    while matches!(lines.last(), Some(line) if line.spans.iter().all(|span| span.content == "")) {
        lines.pop();
    }
}

fn line_plain_text(line: &Line<'static>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>()
}

fn is_progress_stack_title(line: &Line<'static>) -> bool {
    matches!(
        line_plain_text(line).trim(),
        "Plan Mode" | "Exploring" | "Planning" | "Running"
    )
}

fn split_progress_sentences(message: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current = String::new();
    let mut chars = message.chars().peekable();

    while let Some(ch) = chars.next() {
        current.push(ch);

        let next = chars.peek().copied();
        let previous = current.chars().rev().nth(1);
        let is_decimal_separator = ch == '.'
            && previous.is_some_and(|prev| prev.is_ascii_digit())
            && next.is_some_and(|peek| peek.is_ascii_digit());
        let continues_punctuation = next.is_some_and(|peek| matches!(peek, '.' | '!' | '?'));

        if matches!(ch, '.' | '!' | '?') && !is_decimal_separator && !continues_punctuation {
            let trimmed = current.trim();
            if !trimmed.is_empty() {
                sentences.push(trimmed.to_string());
            }
            current.clear();
        }
    }

    let tail = current.trim();
    if !tail.is_empty() {
        sentences.push(tail.to_string());
    }

    sentences
}

fn is_structured_response_marker(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with("<plan>")
        || trimmed.starts_with("</plan>")
        || trimmed.starts_with("<request_user_input>")
        || trimmed.starts_with("</request_user_input>")
        || trimmed.starts_with("<continue_inspection")
}

fn is_structured_progress_list_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("- [") || trimmed.starts_with("* [") || trimmed.starts_with("• [")
}

fn compact_live_response_source(message: &str) -> Option<String> {
    let mut retained = Vec::new();
    let mut saw_prose = false;
    let mut in_structured_block = false;

    for line in message.lines() {
        let trimmed = line.trim();
        if is_structured_response_marker(trimmed) {
            if trimmed.starts_with("</") {
                in_structured_block = false;
            } else if trimmed.starts_with('<') && trimmed.ends_with('>') && !trimmed.ends_with("/>")
            {
                in_structured_block = true;
            }
            continue;
        }

        if in_structured_block {
            continue;
        }

        if trimmed.is_empty() {
            if saw_prose {
                retained.push(String::new());
            }
            continue;
        }

        if is_structured_progress_list_line(trimmed) && saw_prose {
            continue;
        }

        retained.push(trimmed.to_string());
        saw_prose = true;
    }

    let compact = retained
        .into_iter()
        .skip_while(|line| line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();

    if compact.is_empty() {
        None
    } else {
        Some(compact)
    }
}

fn compact_live_response_message(message: &str) -> Option<String> {
    let source = compact_live_response_source(message)?;
    let sentences = split_progress_sentences(&source);
    if sentences.len() <= 3 {
        return Some(sentences.join("\n"));
    }

    let next_markers = [
        "next ",
        "i will ",
        "i'll ",
        "then i will ",
        "then i'll ",
        "i am going to ",
    ];

    let mut selected_indices = vec![0];
    let mut next_step_idx = None;

    for (idx, sentence) in sentences.iter().enumerate().skip(1) {
        let lowered = sentence.to_ascii_lowercase();
        if next_step_idx.is_none()
            && next_markers
                .iter()
                .any(|marker| lowered.starts_with(marker))
        {
            next_step_idx = Some(idx);
            break;
        }
    }

    if let Some(idx) = next_step_idx {
        selected_indices.push(idx);
    }

    let mut idx = 1;
    while selected_indices.len() < 3 && idx < sentences.len() {
        if !selected_indices.contains(&idx) {
            selected_indices.push(idx);
        }
        idx += 1;
    }

    selected_indices.sort_unstable();
    Some(
        selected_indices
            .into_iter()
            .map(|idx| sentences[idx].clone())
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

fn ordered_exploration_agent_segments<'a>(
    current_turn: &[&'a TranscriptEntry],
) -> Option<Vec<OrderedActiveSegment<'a>>> {
    let mut segments = Vec::new();
    let mut exploration_items = Vec::new();
    let mut saw_interleaving = false;

    let flush_exploration = |segments: &mut Vec<OrderedActiveSegment<'a>>,
                             items: &mut Vec<String>| {
        if !items.is_empty() {
            segments.push(OrderedActiveSegment::Exploration(std::mem::take(items)));
        }
    };

    for entry in current_turn {
        match entry.role.as_str() {
            "Tool" => {
                if let Some(action) = super::exploration_action_label(&entry.message) {
                    if !exploration_items.iter().any(|existing| existing == &action) {
                        exploration_items.push(action);
                    }
                }
            }
            "Exploring" => {
                for item in entry
                    .message
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(|line| {
                        line.trim_start_matches("└")
                            .trim_start_matches("•")
                            .trim()
                            .to_string()
                    })
                    .filter(|line| !line.is_empty())
                {
                    if !exploration_items.iter().any(|existing| existing == &item) {
                        exploration_items.push(item);
                    }
                }
            }
            "Agent" => {
                if !exploration_items.is_empty() {
                    saw_interleaving = true;
                    flush_exploration(&mut segments, &mut exploration_items);
                }
                segments.push(OrderedActiveSegment::Agent(entry.message.as_str()));
            }
            _ => {}
        }
    }

    flush_exploration(&mut segments, &mut exploration_items);

    let simple_exploration_then_agent = segments.len() == 2
        && matches!(segments.first(), Some(OrderedActiveSegment::Exploration(_)))
        && matches!(segments.last(), Some(OrderedActiveSegment::Agent(_)));

    if saw_interleaving || (segments.len() > 1 && !simple_exploration_then_agent) {
        Some(segments)
    } else {
        None
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
        let has_tool_activity = entry_refs.iter().any(|entry| {
            matches!(
                entry.role.as_str(),
                "Tool" | "Tool Result" | "Tool Error" | "Tool Progress"
            )
        });
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
        let mut previous_was_progress_stack_title = false;
        for (idx, cell) in cells.into_iter().enumerate() {
            let cell_lines = cell.display_lines(width);
            let current_is_progress_stack_title =
                cell_lines.first().is_some_and(is_progress_stack_title);
            if idx > 0 && !(previous_was_progress_stack_title && current_is_progress_stack_title) {
                lines.push(Line::from(""));
            }
            lines.extend(cell_lines);
            previous_was_progress_stack_title = current_is_progress_stack_title;
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
                    Box::new(PlanningSuggestionCell::new(planning_suggestion_text(
                        self.app,
                    ))),
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
        let has_tool_activity = current_turn.iter().any(|entry| {
            matches!(
                entry.role.as_str(),
                "Tool" | "Tool Result" | "Tool Error" | "Tool Progress"
            )
        });
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

        let ordered_exploration_agent_segments =
            if !has_live_exploration && !has_live_planning && !has_live_running {
                ordered_exploration_agent_segments(current_turn.as_slice())
            } else {
                None
            };
        let uses_ordered_exploration_agent_segments = ordered_exploration_agent_segments.is_some();

        if let Some(segments) = ordered_exploration_agent_segments.as_ref() {
            for segment in segments {
                match segment {
                    OrderedActiveSegment::Exploration(items) => {
                        let summary =
                            compact_summary_lines(items.as_slice(), 4, "more exploration step(s)");
                        cells.push(Box::new(ExploringCell::new(summary, turn_live)));
                    }
                    OrderedActiveSegment::Agent(message) => {
                        cells.push(Box::new(MessageCell::new(
                            "Agent",
                            message,
                            usize::MAX,
                            self.cwd,
                        )));
                    }
                }
            }
        }

        let explicit_exploration = current_turn
            .iter()
            .find(|entry| entry.role == "Exploring")
            .map(|entry| entry.message.clone());

        let exploration_summary = if uses_ordered_exploration_agent_segments {
            None
        } else if has_live_exploration {
            Some(compact_progress_summary_lines(
                self.app.active_live.exploration_actions.as_slice(),
                self.app.active_live.exploration_notes.as_slice(),
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
            Some(compact_progress_summary_lines(
                self.app.active_live.planning_actions.as_slice(),
                self.app.active_live.planning_notes.as_slice(),
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
            Some(compact_recent_first_summary_lines(
                self.app.active_live.running_actions.as_slice(),
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
        let compact_live_response =
            turn_live && (has_exploration_summary || has_planning_summary || has_running_summary);

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
            )
            && !has_exploration_summary
            && !has_planning_summary
            && !has_running_summary
            && self.app.snapshot.plan_steps.is_empty()
            && !suppress_planning_chatter
            && !suppress_structured_plan_response
            && self.app.pending_request_input().is_none()
            && !self.app.has_pending_plan_approval()
            && self.app.pending_command_approval().is_none()
            && self.app.pending_planning_suggestion.is_none();

        if uses_ordered_exploration_agent_segments {
            // Preserve chronological "explore -> agent -> explore" segments without
            // reintroducing the latest agent/tool fallback below.
        } else if has_agent_stream
            && !suppress_intermediate_agent
            && !suppress_planning_chatter
            && !suppress_structured_plan_response
        {
            if let Some(stream_lines) = streaming_agent_lines {
                if compact_live_response {
                    cells.push(Box::new(RespondingCell::from_stream_compact(
                        stream_lines,
                        4,
                    )));
                } else {
                    cells.push(Box::new(RespondingCell::from_stream(stream_lines)));
                }
            } else if let Some(agent_message) = latest_agent {
                if compact_live_response {
                    if let Some(message) = compact_live_response_message(agent_message) {
                        cells.push(Box::new(RespondingCell::from_compact_message(
                            message,
                            usize::MAX,
                        )));
                    }
                } else {
                    cells.push(Box::new(RespondingCell::from_message(
                        responding_role,
                        agent_message,
                        usize::MAX,
                        self.cwd,
                    )));
                }
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
            if compact_live_response {
                if let Some(message) = compact_live_response_message(agent_message) {
                    cells.push(Box::new(RespondingCell::from_compact_message(
                        message,
                        usize::MAX,
                    )));
                }
            } else {
                cells.push(Box::new(RespondingCell::from_message(
                    responding_role,
                    agent_message,
                    usize::MAX,
                    self.cwd,
                )));
            }
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
        let mut previous_was_progress_stack_title = false;
        for (idx, cell) in cells.into_iter().enumerate() {
            let cell_lines = cell.display_lines(width);
            let current_is_progress_stack_title =
                cell_lines.first().is_some_and(is_progress_stack_title);
            if idx > 0 && !(previous_was_progress_stack_title && current_is_progress_stack_title) {
                lines.push(Line::from(""));
            }
            lines.extend(cell_lines);
            previous_was_progress_stack_title = current_is_progress_stack_title;
        }

        trim_trailing_empty_lines(&mut lines);
        lines
    }
}

#[cfg(test)]
mod helper_tests {
    use super::{
        compact_live_response_message, compact_live_response_source, split_progress_sentences,
    };

    #[test]
    fn split_progress_sentences_keeps_ellipses_and_decimal_versions() {
        let sentences = split_progress_sentences(
            "Wait... I checked v1.0 parsing. Next I will inspect restore.",
        );

        assert_eq!(
            sentences,
            vec![
                "Wait...".to_string(),
                "I checked v1.0 parsing.".to_string(),
                "Next I will inspect restore.".to_string(),
            ]
        );
    }

    #[test]
    fn compact_live_response_message_preserves_selected_sentence_order() {
        let rendered = compact_live_response_message(
            "Next I will inspect restore. I checked the auth path. I checked the persistence path. Then I will verify chronology.",
        )
        .unwrap();

        assert_eq!(
            rendered,
            [
                "Next I will inspect restore.",
                "I checked the auth path.",
                "Then I will verify chronology.",
            ]
            .join("\n")
        );
    }

    #[test]
    fn compact_live_response_source_strips_structured_plan_block() {
        let rendered = compact_live_response_source(
            "我先对比代码结构和现有 todo，找出未记录但值得改进的点。\n我再补两处证据，尽量把建议落到具体代码位置。\n<plan>\n- [completed] Inspect runtime entrypoint\n- [pending] Tighten render path\n</plan>",
        )
        .unwrap();

        assert_eq!(
            rendered,
            "我先对比代码结构和现有 todo，找出未记录但值得改进的点。\n我再补两处证据，尽量把建议落到具体代码位置。"
        );
    }

    #[test]
    fn compact_live_response_source_drops_checklist_tail_after_prose() {
        let rendered = compact_live_response_source(
            "I inspected the current context path.\nI will reuse the existing assembler output.\n- [completed] Review context/runtime.rs\n- [pending] Add a focused test",
        )
        .unwrap();

        assert_eq!(
            rendered,
            "I inspected the current context path.\nI will reuse the existing assembler output."
        );
    }

    #[test]
    fn compact_live_response_source_keeps_prose_after_structured_plan_block() {
        let rendered = compact_live_response_source(
            "I inspected the current context path.\n<plan>\n- [completed] Review context/runtime.rs\n- [pending] Add a focused test\n</plan>\nI am starting the focused patch now.",
        )
        .unwrap();

        assert_eq!(
            rendered,
            "I inspected the current context path.\nI am starting the focused patch now."
        );
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
