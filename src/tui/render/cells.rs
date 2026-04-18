use std::path::Path;

use ratatui::{
    style::Color,
    text::{Line, Span},
};

use crate::tui::command::{
    status_plan_text, status_planning_suggestion_text, status_request_user_input_text,
};
use crate::tui::state::{
    contains_structured_planning_output, RuntimePhase, TranscriptEntry, TuiApp,
};

use super::{
    current_turn_exploration_summary, current_turn_exploration_summary_from_entries,
    current_turn_tool_summary, formatted_message_lines, prefixed_message_lines,
    rendered_markdown_lines, section_span, with_border, wrapped_history_line_count,
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

struct UserCell {
    message: String,
}

impl UserCell {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl HistoryCell for UserCell {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        prefixed_message_lines("You", &self.message, 4)
    }
}

struct SummaryCell {
    title: &'static str,
    color: Color,
    summary: String,
}

impl SummaryCell {
    fn new(title: &'static str, color: Color, summary: impl Into<String>) -> Self {
        Self {
            title,
            color,
            summary: summary.into(),
        }
    }
}

impl HistoryCell for SummaryCell {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        let mut lines = vec![Line::from(section_span(self.title, self.color))];
        lines.extend(
            self.summary
                .lines()
                .map(|line| Line::from(format!("  {line}"))),
        );
        lines
    }
}

struct ExploredCell {
    inner: SummaryCell,
}

impl ExploredCell {
    fn new(summary: impl Into<String>) -> Self {
        Self {
            inner: SummaryCell::new("Explored", Color::Rgb(231, 201, 92), summary),
        }
    }
}

impl HistoryCell for ExploredCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.inner.display_lines(width)
    }
}

struct RanCell {
    inner: SummaryCell,
}

impl RanCell {
    fn new(summary: impl Into<String>) -> Self {
        Self {
            inner: SummaryCell::new("Ran", Color::LightYellow, summary),
        }
    }
}

impl HistoryCell for RanCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.inner.display_lines(width)
    }
}

struct PlanSummaryCell {
    inner: SummaryCell,
}

impl PlanSummaryCell {
    fn new(summary: impl Into<String>) -> Self {
        Self {
            inner: SummaryCell::new("Plan", Color::LightBlue, summary),
        }
    }
}

impl HistoryCell for PlanSummaryCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.inner.display_lines(width)
    }
}

struct PlanningCell {
    inner: SummaryCell,
}

impl PlanningCell {
    fn new(summary: impl Into<String>, active: bool) -> Self {
        let (title, color) = if active {
            ("Planning", Color::LightBlue)
        } else {
            ("Planned", Color::LightBlue)
        };
        Self {
            inner: SummaryCell::new(title, color, summary),
        }
    }
}

impl HistoryCell for PlanningCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.inner.display_lines(width)
    }
}

struct ExploringCell {
    inner: SummaryCell,
}

impl ExploringCell {
    fn new(summary: impl Into<String>, active: bool) -> Self {
        let (title, color) = if active {
            ("Exploring", Color::Yellow)
        } else {
            ("Explored", Color::Rgb(231, 201, 92))
        };
        Self {
            inner: SummaryCell::new(title, color, summary),
        }
    }
}

impl HistoryCell for ExploringCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.inner.display_lines(width)
    }
}

struct RunningCell {
    inner: SummaryCell,
}

impl RunningCell {
    fn new(summary: impl Into<String>, active: bool) -> Self {
        let (title, color) = if active {
            ("Running", Color::Yellow)
        } else {
            ("Ran", Color::LightYellow)
        };
        Self {
            inner: SummaryCell::new(title, color, summary),
        }
    }
}

impl HistoryCell for RunningCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.inner.display_lines(width)
    }
}

struct ApprovalCell {
    title: &'static str,
    color: Color,
    lines: Vec<String>,
}

impl ApprovalCell {
    fn new(title: &'static str, color: Color, lines: Vec<String>) -> Self {
        Self {
            title,
            color,
            lines,
        }
    }
}

impl HistoryCell for ApprovalCell {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        let mut lines = vec![Line::from(section_span(self.title, self.color))];
        lines.extend(
            self.lines
                .iter()
                .map(|line| Line::from(format!("  {line}"))),
        );
        lines
    }
}

struct PlanningSuggestionCell {
    text: String,
}

impl PlanningSuggestionCell {
    fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }
}

impl HistoryCell for PlanningSuggestionCell {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        let mut lines = vec![Line::from(section_span(
            "Planning Suggested",
            Color::LightBlue,
        ))];
        lines.extend(
            self.text
                .lines()
                .map(|line| Line::from(format!("  {line}"))),
        );
        lines
    }
}

struct CompletionCell {
    title: &'static str,
    color: Color,
    summary: String,
}

impl CompletionCell {
    fn new(title: &'static str, color: Color, summary: impl Into<String>) -> Self {
        Self {
            title,
            color,
            summary: summary.into(),
        }
    }
}

impl HistoryCell for CompletionCell {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        vec![
            Line::from(section_span(self.title, self.color)),
            Line::from(format!("  {}", self.summary)),
        ]
    }
}

struct PlanModeCell;

impl HistoryCell for PlanModeCell {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        vec![Line::from(section_span("Plan Mode", Color::LightBlue))]
    }
}

struct RespondingCell<'a> {
    content: RespondingCellContent<'a>,
}

enum RespondingCellContent<'a> {
    Stream(&'a [Line<'static>]),
    Message {
        role: &'static str,
        message: &'a str,
        max_lines: usize,
        cwd: Option<&'a Path>,
    },
    ToolResult {
        role: &'a str,
        message: &'a str,
        max_lines: usize,
    },
    Working(&'a str),
}

impl<'a> RespondingCell<'a> {
    fn from_stream(stream_lines: &'a [Line<'static>]) -> Self {
        Self {
            content: RespondingCellContent::Stream(stream_lines),
        }
    }

    fn from_message(
        role: &'static str,
        message: &'a str,
        max_lines: usize,
        cwd: Option<&'a Path>,
    ) -> Self {
        Self {
            content: RespondingCellContent::Message {
                role,
                message,
                max_lines,
                cwd,
            },
        }
    }

    fn from_tool_result(role: &'a str, message: &'a str, max_lines: usize) -> Self {
        Self {
            content: RespondingCellContent::ToolResult {
                role,
                message,
                max_lines,
            },
        }
    }

    fn working(detail: &'a str) -> Self {
        Self {
            content: RespondingCellContent::Working(detail),
        }
    }
}

impl HistoryCell for RespondingCell<'_> {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        match &self.content {
            RespondingCellContent::Stream(stream_lines) => {
                rendered_markdown_lines("Responding", stream_lines, usize::MAX)
            }
            RespondingCellContent::Message {
                role,
                message,
                max_lines,
                cwd,
            } => formatted_message_lines(role, message, *max_lines, *cwd),
            RespondingCellContent::ToolResult {
                role,
                message,
                max_lines,
            } => prefixed_message_lines(role, message, *max_lines),
            RespondingCellContent::Working(detail) => vec![
                Line::from(section_span("Working", Color::Yellow)),
                Line::from(format!("  {detail}")),
            ],
        }
    }
}

struct MessageCell<'a> {
    role: &'a str,
    message: &'a str,
    max_lines: usize,
    cwd: Option<&'a Path>,
}

impl<'a> MessageCell<'a> {
    fn new(role: &'a str, message: &'a str, max_lines: usize, cwd: Option<&'a Path>) -> Self {
        Self {
            role,
            message,
            max_lines,
            cwd,
        }
    }
}

impl HistoryCell for MessageCell<'_> {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        formatted_message_lines(self.role, self.message, self.max_lines, self.cwd)
    }
}

pub(crate) struct StartupCardCell {
    model_label: String,
    directory: String,
}

impl StartupCardCell {
    pub(crate) fn new(model_label: String, directory: String) -> Self {
        Self {
            model_label,
            directory,
        }
    }
}

impl HistoryCell for StartupCardCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let Some(inner_width) = super::startup_card_inner_width(width) else {
            return Vec::new();
        };

        let model_label = "model:";
        let directory_label = "directory:";
        let label_width = directory_label.len();
        let model_prefix = format!("{model_label:<label_width$} ");
        let hint = "/model to change";
        let hint_width = super::display_width(hint);
        let model_prefix_width = super::display_width(&model_prefix);
        let model_available_width = inner_width
            .saturating_sub(model_prefix_width)
            .saturating_sub(1)
            .saturating_sub(hint_width);
        let model_value =
            super::truncate_for_startup_card(&self.model_label, model_available_width);
        let model_value_width = super::display_width(&model_value);
        let gap_width = inner_width
            .saturating_sub(model_prefix_width)
            .saturating_sub(model_value_width)
            .saturating_sub(hint_width)
            .max(1);
        let directory_prefix = format!("{directory_label:<label_width$} ");
        let directory_max_width =
            inner_width.saturating_sub(super::display_width(&directory_prefix));

        let lines = vec![
            Line::from(vec![Span::from(">_ "), Span::from("RARA")]),
            Line::from(""),
            Line::from(vec![
                Span::from(model_prefix),
                Span::from(model_value),
                Span::from(" ".repeat(gap_width)),
                Span::from(hint),
            ]),
            Line::from(vec![
                Span::from(directory_prefix),
                Span::from(super::truncate_path_middle(
                    &self.directory,
                    directory_max_width,
                )),
            ]),
        ];

        with_border(lines, inner_width)
    }
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
        let has_tool_activity = entry_refs
            .iter()
            .any(|entry| matches!(entry.role.as_str(), "Tool" | "Tool Result" | "Tool Error"));
        if let Some(summary) =
            current_turn_exploration_summary_from_entries(entry_refs.as_slice(), false, None)
        {
            cells.push(Box::new(ExploredCell::new(summary)));
        }

        if let Some(summary) = current_turn_tool_summary(entry_refs.as_slice(), false, None) {
            cells.push(Box::new(RanCell::new(summary)));
        }

        let tail_entries: Vec<&TranscriptEntry> = if has_tool_activity {
            self.entries
                .iter()
                .rev()
                .filter(|entry| {
                    entry.role == "Agent"
                        || (entry.role == "System"
                            && is_renderable_system_message(entry.message.as_str()))
                })
                .take(1)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect()
        } else {
            self.entries
                .iter()
                .filter(|entry| {
                    entry.role == "Agent"
                        || (entry.role == "System"
                            && is_renderable_system_message(entry.message.as_str()))
                })
                .collect()
        };

        for entry in tail_entries {
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
                        status_planning_suggestion_text(self.app),
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
            .any(|entry| matches!(entry.role.as_str(), "Tool" | "Tool Result" | "Tool Error"));
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
            .find(|entry| entry.role == "Tool Result" || entry.role == "Tool Error")
            .map(|entry| (entry.role.as_str(), entry.message.as_str()));
        let mut cells: Vec<Box<dyn HistoryCell + '_>> = Vec::new();
        let has_live_exploration = !self.app.active_live.exploration_actions.is_empty()
            || !self.app.active_live.exploration_notes.is_empty();
        let has_live_planning = !self.app.active_live.planning_actions.is_empty();
        let has_live_running = !self.app.active_live.running_actions.is_empty();

        if !user_message.is_empty() {
            cells.push(Box::new(UserCell::new(user_message)));
        }

        if self.app.agent_execution_mode_label() == "plan" {
            cells.push(Box::new(PlanModeCell));
        }

        let exploration_summary = if has_live_exploration {
            let mut lines = self
                .app
                .active_live
                .exploration_actions
                .iter()
                .map(|action| format!("└ {action}"))
                .collect::<Vec<_>>();
            lines.extend(
                self.app
                    .active_live
                    .exploration_notes
                    .iter()
                    .map(|note| format!("└ {note}")),
            );
            if turn_live {
                lines.push(format!(
                    "└ {}",
                    self.app
                        .runtime_phase_detail
                        .as_deref()
                        .unwrap_or("waiting for more exploration output")
                ));
            }
            Some(lines.join("\n"))
        } else {
            current_turn_exploration_summary(self.app, current_turn.as_slice(), turn_live)
        };
        let has_exploration_summary = exploration_summary.is_some();
        let exploration_active = turn_live && has_exploration_summary;
        if let Some(summary) = exploration_summary {
            cells.push(Box::new(ExploringCell::new(summary, exploration_active)));
        }

        let planning_summary = if has_live_planning {
            let mut lines = self
                .app
                .active_live
                .planning_actions
                .iter()
                .map(|action| format!("└ {action}"))
                .collect::<Vec<_>>();
            if turn_live {
                lines.push(format!(
                    "└ {}",
                    self.app
                        .runtime_phase_detail
                        .as_deref()
                        .unwrap_or("waiting for plan output")
                ));
            }
            Some(lines.join("\n"))
        } else {
            None
        };
        if let Some(summary) = planning_summary {
            cells.push(Box::new(PlanningCell::new(summary, turn_live)));
        }

        let running_summary = if has_live_running {
            let mut lines = self
                .app
                .active_live
                .running_actions
                .iter()
                .map(|action| format!("└ {action}"))
                .collect::<Vec<_>>();
            if turn_live {
                lines.push(format!(
                    "└ {}",
                    self.app
                        .runtime_phase_detail
                        .as_deref()
                        .unwrap_or("waiting for tool output")
                ));
            }
            Some(lines.join("\n"))
        } else {
            current_turn_tool_summary(
                current_turn.as_slice(),
                turn_live,
                self.app.runtime_phase_detail.as_deref(),
            )
        };
        let running_active = turn_live && running_summary.is_some();
        if let Some(summary) = running_summary {
            cells.push(Box::new(RunningCell::new(summary, running_active)));
        }

        if !self.app.snapshot.plan_steps.is_empty() {
            cells.push(Box::new(PlanSummaryCell::new(
                status_plan_text(self.app)
                    .lines()
                    .take(8)
                    .collect::<Vec<_>>()
                    .join("\n"),
            )));
        }

        if self.app.snapshot.pending_question.is_some() {
            let (title, color) = if self.app.has_pending_approval() {
                ("Approval", Color::Yellow)
            } else {
                ("Request Input", Color::LightGreen)
            };
            let mut request_lines = status_request_user_input_text(self.app)
                .lines()
                .take(8)
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            request_lines.push("shortcuts: press 1/2/3 to answer immediately".to_string());
            cells.push(Box::new(ApprovalCell::new(title, color, request_lines)));
        }

        if self.app.pending_planning_suggestion.is_some() {
            cells.push(Box::new(PlanningSuggestionCell::new(
                status_planning_suggestion_text(self.app),
            )));
        }

        if let Some((title, summary)) = self.app.snapshot.completed_approval.as_ref() {
            cells.push(Box::new(CompletionCell::new(
                "Approval Completed",
                Color::LightGreen,
                format!("{title}: {summary}"),
            )));
        }

        if let Some((title, summary)) = self.app.snapshot.completed_question.as_ref() {
            cells.push(Box::new(CompletionCell::new(
                "Question Answered",
                Color::LightGreen,
                format!("{title}: {summary}"),
            )));
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
            && self.app.snapshot.pending_question.is_none()
            && !self.app.has_pending_plan_approval();

        let responding_role = if turn_live { "Responding" } else { "Agent" };

        if let Some(stream_lines) = streaming_agent_lines
            .filter(|_| !suppress_intermediate_agent && !suppress_planning_chatter)
        {
            cells.push(Box::new(RespondingCell::from_stream(stream_lines)));
        } else if let Some(agent_message) =
            latest_agent.filter(|_| !suppress_intermediate_agent && !suppress_planning_chatter)
        {
            cells.push(Box::new(RespondingCell::from_message(
                responding_role,
                agent_message,
                usize::MAX,
                self.cwd,
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
        } else if turn_live {
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
        || lower.starts_with("error:")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::config::ConfigManager;
    use crate::tui::state::{
        RuntimePhase, RuntimeSnapshot, TranscriptEntry, TranscriptTurn, TuiApp,
    };
    use tempfile::tempdir;

    use super::{ActiveCell, ActiveTurnCell, CommittedTurnCell, HistoryCell};

    #[test]
    fn committed_turn_cell_keeps_user_summary_and_agent_sections_in_order() {
        let entries = vec![
            TranscriptEntry {
                role: "You".into(),
                message: "Review this repo".into(),
            },
            TranscriptEntry {
                role: "Tool".into(),
                message: "list_files .".into(),
            },
            TranscriptEntry {
                role: "Tool".into(),
                message: "bash cargo check".into(),
            },
            TranscriptEntry {
                role: "Agent".into(),
                message: "Final recommendation".into(),
            },
        ];

        let rendered = CommittedTurnCell::new(entries.as_slice(), Some(Path::new(".")))
            .display_lines(100)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        let you_idx = rendered.find("You: Review this repo").unwrap();
        let explored_idx = rendered.find(" Explored ").unwrap();
        let ran_idx = rendered.find(" Ran ").unwrap();
        let agent_idx = rendered.find("Agent\n  Final recommendation").unwrap();

        assert!(you_idx < explored_idx);
        assert!(explored_idx < ran_idx);
        assert!(ran_idx < agent_idx);
    }

    #[test]
    fn active_turn_cell_keeps_sections_in_stable_order() {
        let temp = tempdir().unwrap();
        let mut app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("build tui app");
        app.runtime_phase = RuntimePhase::RunningTool;
        app.runtime_phase_detail = Some("waiting for tool output".into());
        app.active_turn = TranscriptTurn {
            entries: vec![
                TranscriptEntry {
                    role: "You".into(),
                    message: "Inspect the codebase".into(),
                },
                TranscriptEntry {
                    role: "Tool".into(),
                    message: "list_files src".into(),
                },
                TranscriptEntry {
                    role: "Tool".into(),
                    message: "bash cargo check".into(),
                },
            ],
        };
        app.snapshot = RuntimeSnapshot {
            plan_steps: vec![("pending".into(), "Review architecture".into())],
            pending_question: Some((
                "Approve plan".into(),
                vec![("1".into(), "Implement".into())],
                None,
            )),
            ..RuntimeSnapshot::default()
        };

        let rendered = ActiveTurnCell::new(&app, Some(Path::new(".")))
            .display_lines(100)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        let you_idx = rendered.find("You: Inspect the codebase").unwrap();
        let exploring_idx = rendered.find(" Exploring ").unwrap();
        let running_idx = rendered.find(" Running ").unwrap();
        let plan_idx = rendered.find(" Plan ").unwrap();
        let approval_idx = rendered.find(" Request Input ").unwrap();

        assert!(you_idx < exploring_idx);
        assert!(exploring_idx < running_idx);
        assert!(running_idx < plan_idx);
        assert!(plan_idx < approval_idx);
    }

    #[test]
    fn active_turn_cell_renders_planning_suggestion_without_active_turn_entries() {
        let temp = tempdir().unwrap();
        let mut app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("build tui app");
        app.queue_planning_suggestion("Review this repository and propose changes.");

        let rendered = ActiveTurnCell::new(&app, Some(Path::new(".")))
            .display_lines(100)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("You: Review this repository and propose changes."));
        assert!(rendered.contains(" Planning Suggested "));
        assert!(rendered.contains("Enter planning mode"));
        assert!(rendered.contains("Continue in execute mode"));
    }

    #[test]
    fn active_turn_cell_keeps_exploration_notes_inside_exploring_block() {
        let temp = tempdir().unwrap();
        let mut app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("build tui app");
        app.runtime_phase = RuntimePhase::RunningTool;
        app.runtime_phase_detail = Some("waiting for model response · 12s elapsed".into());
        app.active_turn = TranscriptTurn {
            entries: vec![
                TranscriptEntry {
                    role: "You".into(),
                    message: "Review this repository".into(),
                },
                TranscriptEntry {
                    role: "Tool".into(),
                    message: "read_file src/main.rs".into(),
                },
                TranscriptEntry {
                    role: "Agent".into(),
                    message:
                        "I have inspected the repository structure and will now inspect the core modules."
                            .into(),
                },
            ],
        };

        let rendered = ActiveTurnCell::new(&app, Some(Path::new(".")))
            .display_lines(100)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            rendered.contains(" Exploring "),
            "rendered_exploration_notes=\n{rendered}"
        );
        assert!(rendered.contains("Read src/main.rs"));
        assert!(rendered.contains(
            "I have inspected the repository structure and will now inspect the core modules."
        ));
        assert!(rendered.contains("waiting for model response · 12s elapsed"));
    }

    #[test]
    fn active_turn_cell_uses_stateful_live_exploration_sections() {
        let temp = tempdir().unwrap();
        let mut app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("build tui app");
        app.runtime_phase = RuntimePhase::RunningTool;
        app.runtime_phase_detail = Some("waiting for model response · 20s elapsed".into());
        app.active_turn = TranscriptTurn {
            entries: vec![TranscriptEntry {
                role: "You".into(),
                message: "Inspect the repository".into(),
            }],
        };
        app.record_exploration_action("Read src/tools/vector.rs");
        app.record_exploration_note("I have inspected the repository structure.");

        let rendered = ActiveTurnCell::new(&app, Some(Path::new(".")))
            .display_lines(100)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            rendered.contains(" Exploring "),
            "rendered_stateful_exploration=\n{rendered}"
        );
        assert!(rendered.contains("Read src/tools/vector.rs"));
        assert!(rendered.contains("I have inspected the repository structure."));
        assert!(rendered.contains("waiting for model response · 20s elapsed"));
    }

    #[test]
    fn active_turn_cell_suppresses_planning_chatter_when_exploring() {
        let temp = tempdir().unwrap();
        let mut app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("build tui app");
        app.agent_execution_mode = crate::agent::AgentExecutionMode::Plan;
        app.runtime_phase = RuntimePhase::ProcessingResponse;
        app.active_turn = TranscriptTurn {
            entries: vec![
                TranscriptEntry {
                    role: "You".into(),
                    message: "Review this repository".into(),
                },
                TranscriptEntry {
                    role: "Agent".into(),
                    message:
                        "I will now read crates/instructions/src/prompt.rs to continue the review."
                            .into(),
                },
            ],
        };
        app.record_exploration_action("Read crates/instructions/src/lib.rs");

        let rendered = ActiveTurnCell::new(&app, Some(Path::new(".")))
            .display_lines(100)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains(" Plan Mode "));
        assert!(rendered.contains(" Exploring "));
        assert!(!rendered.contains("Responding"));
        assert!(!rendered.contains("I will now read crates/instructions/src/prompt.rs"));
    }

    #[test]
    fn active_turn_cell_uses_responding_label_while_busy() {
        let temp = tempdir().unwrap();
        let mut app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("build tui app");
        app.runtime_phase = RuntimePhase::ProcessingResponse;
        app.runtime_phase_detail = Some("waiting for model response · 2s elapsed".into());
        app.active_turn = TranscriptTurn {
            entries: vec![
                TranscriptEntry {
                    role: "You".into(),
                    message: "Review this repository".into(),
                },
                TranscriptEntry {
                    role: "Agent".into(),
                    message: "I have inspected the main module and will continue with the tool layer."
                        .into(),
                },
            ],
        };

        let rendered = ActiveTurnCell::new(&app, Some(Path::new(".")))
            .display_lines(100)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("Responding"));
        assert!(!rendered.contains("Agent\n  I have inspected"));
    }

    #[test]
    fn active_turn_cell_shows_planning_section_for_plan_agent() {
        let temp = tempdir().unwrap();
        let mut app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("build tui app");
        app.runtime_phase = RuntimePhase::RunningTool;
        app.runtime_phase_detail = Some("plan_agent {\"instruction\":\"refine the plan\"}".into());
        app.active_turn = TranscriptTurn {
            entries: vec![TranscriptEntry {
                role: "You".into(),
                message: "Plan the refactor".into(),
            }],
        };
        app.record_planning_action("Delegate plan refinement: refine the plan");

        let rendered = ActiveTurnCell::new(&app, Some(Path::new(".")))
            .display_lines(100)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains(" Planning "));
        assert!(rendered.contains("Delegate plan refinement: refine the plan"));
    }

    #[test]
    fn committed_turn_cell_ignores_routine_system_notices() {
        let entries = vec![
            TranscriptEntry {
                role: "You".into(),
                message: "Review this repo".into(),
            },
            TranscriptEntry {
                role: "Agent".into(),
                message: "Final recommendation".into(),
            },
            TranscriptEntry {
                role: "System".into(),
                message: "prompt finished".into(),
            },
        ];

        let rendered = CommittedTurnCell::new(entries.as_slice(), Some(Path::new(".")))
            .display_lines(100)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("Agent\n  Final recommendation"));
        assert!(!rendered.contains("System"));
        assert!(!rendered.contains("prompt finished"));
    }
}
