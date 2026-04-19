use std::path::Path;

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use crate::tui::interaction_text::{
    pending_interaction_card_title, pending_interaction_detail_text,
    pending_interaction_shortcut_text, status_planning_suggestion_text,
};
use crate::tui::plan_display::updated_plan_lines;
use crate::tui::state::{
    contains_structured_planning_output, ActivePendingInteractionKind, InteractionKind,
    RuntimePhase, TranscriptEntry, TuiApp,
};

use super::{
    current_turn_exploration_summary, current_turn_exploration_summary_from_entries,
    current_turn_tool_summary, formatted_message_lines,
    history_pipeline::{narrative_entries, ordered_completion_entries},
    prefixed_message_lines, rendered_markdown_lines, section_span, with_border,
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
enum InteractionCompletionKind {
    ShellApprovalCompleted,
    PlanDecision,
    QuestionAnswered,
    PlanningQuestionAnswered,
    ExplorationQuestionAnswered,
    SubAgentQuestionAnswered,
}

impl InteractionCompletionKind {
    fn from_completed_interaction(kind: InteractionKind, source: Option<&str>) -> Self {
        match kind {
            InteractionKind::Approval => Self::ShellApprovalCompleted,
            InteractionKind::PlanApproval => Self::PlanDecision,
            InteractionKind::RequestInput => match source {
                Some("plan_agent") => Self::PlanningQuestionAnswered,
                Some("explore_agent") => Self::ExplorationQuestionAnswered,
                Some(_) => Self::SubAgentQuestionAnswered,
                None => Self::QuestionAnswered,
            },
        }
    }

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
            Self::PlanDecision => Color::LightBlue,
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
    steps: Vec<(String, String)>,
    explanation: Option<String>,
}

impl PlanSummaryCell {
    fn new(steps: Vec<(String, String)>, explanation: Option<String>) -> Self {
        Self { steps, explanation }
    }
}

impl HistoryCell for PlanSummaryCell {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        updated_plan_lines(self.steps.as_slice(), self.explanation.as_deref())
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

struct PendingInteractionCell {
    inner: ApprovalCell,
}

impl PendingInteractionCell {
    fn new(kind: ActivePendingInteractionKind, lines: Vec<String>) -> Self {
        let color = match kind {
            ActivePendingInteractionKind::PlanApproval
            | ActivePendingInteractionKind::PlanningQuestion => Color::LightBlue,
            ActivePendingInteractionKind::ShellApproval
            | ActivePendingInteractionKind::ExplorationQuestion => Color::Yellow,
            ActivePendingInteractionKind::SubAgentQuestion
            | ActivePendingInteractionKind::RequestInput => Color::LightGreen,
        };
        Self {
            inner: ApprovalCell::new(pending_interaction_card_title(kind), color, lines),
        }
    }
}

impl HistoryCell for PendingInteractionCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.inner.display_lines(width)
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
    title: String,
    color: Color,
    summary: String,
}

impl CompletionCell {
    fn new(title: impl Into<String>, color: Color, summary: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            color,
            summary: summary.into(),
        }
    }
}

struct CommittedInteractionCell {
    inner: CompletionCell,
}

impl CommittedInteractionCell {
    fn new(kind: InteractionCompletionKind, summary: impl Into<String>) -> Self {
        Self {
            inner: CompletionCell::new(kind.title(), kind.color(), summary),
        }
    }
}

impl HistoryCell for CommittedInteractionCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.inner.display_lines(width)
    }
}

impl HistoryCell for CompletionCell {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        vec![
            Line::from(Span::styled(
                format!(" {} ", self.title),
                Style::default()
                    .fg(Color::Black)
                    .bg(self.color)
                    .add_modifier(Modifier::BOLD),
            )),
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
            .any(|entry| matches!(entry.role.as_str(), "Tool" | "Tool Result" | "Tool Error"));
        if let Some(summary) = explicit_exploration.or_else(|| {
            current_turn_exploration_summary_from_entries(entry_refs.as_slice(), false, None)
        }) {
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

        if self.app.agent_execution_mode_label() == "plan" {
            cells.push(Box::new(PlanModeCell));
        }

        let explicit_exploration = current_turn
            .iter()
            .find(|entry| entry.role == "Exploring")
            .map(|entry| entry.message.clone());

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
            explicit_exploration.or_else(|| {
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
            let mut lines = self
                .app
                .active_live
                .planning_actions
                .iter()
                .map(|action| format!("└ {action}"))
                .collect::<Vec<_>>();
            lines.extend(
                self.app
                    .active_live
                    .planning_notes
                    .iter()
                    .map(|note| format!("└ {note}")),
            );
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
            explicit_planning
        };
        if let Some(summary) = planning_summary {
            cells.push(Box::new(PlanningCell::new(summary, turn_live)));
        }

        let explicit_running = current_turn
            .iter()
            .find(|entry| entry.role == "Running")
            .map(|entry| entry.message.clone());

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
            explicit_running.or_else(|| {
                current_turn_tool_summary(
                    current_turn.as_slice(),
                    turn_live,
                    self.app.runtime_phase_detail.as_deref(),
                )
            })
        };
        let running_active = turn_live && running_summary.is_some();
        if let Some(summary) = running_summary {
            cells.push(Box::new(RunningCell::new(summary, running_active)));
        }

        if !self.app.snapshot.plan_steps.is_empty() {
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
                status_planning_suggestion_text(self.app),
            )));
        }

        if latest_completion.is_none() {
            if let Some(interaction) = self
                .app
                .completed_interaction(crate::tui::state::InteractionKind::Approval)
            {
                cells.push(Box::new(CommittedInteractionCell::new(
                    InteractionCompletionKind::from_completed_interaction(
                        InteractionKind::Approval,
                        interaction.source.as_deref(),
                    ),
                    format!("{}: {}", interaction.title, interaction.summary),
                )));
            }

            if let Some(interaction) = self
                .app
                .completed_interaction(crate::tui::state::InteractionKind::RequestInput)
            {
                cells.push(Box::new(CommittedInteractionCell::new(
                    InteractionCompletionKind::from_completed_interaction(
                        InteractionKind::RequestInput,
                        interaction.source.as_deref(),
                    ),
                    format!("{}: {}", interaction.title, interaction.summary),
                )));
            }

            if let Some(interaction) = self
                .app
                .completed_interaction(crate::tui::state::InteractionKind::PlanApproval)
            {
                cells.push(Box::new(CommittedInteractionCell::new(
                    InteractionCompletionKind::from_completed_interaction(
                        InteractionKind::PlanApproval,
                        interaction.source.as_deref(),
                    ),
                    format!("{}: {}", interaction.title, interaction.summary),
                )));
            }
        } else if let Some(entry) = latest_completion {
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
            pending_interactions: vec![crate::tui::state::PendingInteractionSnapshot {
                kind: crate::tui::state::InteractionKind::RequestInput,
                title: "Approve plan".into(),
                summary: String::new(),
                options: vec![("1".into(), "Implement".into())],
                note: None,
                approval: None,
                source: None,
            }],
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
        let plan_idx = rendered.find("Updated Plan").unwrap();
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
    fn active_turn_cell_uses_planning_sidecar_for_non_structured_plan_output() {
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
                    message: "Read the local codebase and suggest improvements".into(),
                },
                TranscriptEntry {
                    role: "Planning".into(),
                    message: "The current discovery is hardcoded to root-level markdown files."
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

        assert!(rendered.contains(" Planning "));
        assert!(rendered.contains("The current discovery is hardcoded"));
        assert!(!rendered.contains("Responding"));
    }

    #[test]
    fn active_turn_cell_uses_explicit_sidecar_entries_when_live_state_is_empty() {
        let temp = tempdir().unwrap();
        let mut app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("build tui app");
        app.runtime_phase = RuntimePhase::ProcessingResponse;
        app.active_turn = TranscriptTurn {
            entries: vec![
                TranscriptEntry {
                    role: "You".into(),
                    message: "Inspect and summarize the repository".into(),
                },
                TranscriptEntry {
                    role: "Exploring".into(),
                    message: "└ Read crates/instructions/src/workspace.rs".into(),
                },
                TranscriptEntry {
                    role: "Planning".into(),
                    message: "The instruction discovery is still root-name based.".into(),
                },
                TranscriptEntry {
                    role: "Running".into(),
                    message: "└ waiting for model response".into(),
                },
            ],
        };

        let rendered = ActiveTurnCell::new(&app, Some(Path::new(".")))
            .display_lines(100)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains(" Exploring "));
        assert!(rendered.contains(" Planning "));
        assert!(rendered.contains(" Running "));
        assert!(rendered.contains("Read crates/instructions/src/workspace.rs"));
        assert!(rendered.contains("root-name based"));
        assert!(rendered.contains("waiting for model response"));
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
                    message:
                        "I have inspected the main module and will continue with the tool layer."
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
        app.record_planning_note("Sub-agent summary: reuse the workspace traversal helper");

        let rendered = ActiveTurnCell::new(&app, Some(Path::new(".")))
            .display_lines(100)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains(" Planning "));
        assert!(rendered.contains("Delegate plan refinement: refine the plan"));
        assert!(rendered.contains("Sub-agent summary: reuse the workspace traversal helper"));
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

    #[test]
    fn committed_turn_cell_renders_materialized_sidecar_sections() {
        let entries = vec![
            TranscriptEntry {
                role: "You".into(),
                message: "Review the workspace logic".into(),
            },
            TranscriptEntry {
                role: "Exploring".into(),
                message: "Delegate repository exploration: inspect instruction discovery\nSub-agent summary: current discovery is hardcoded".into(),
            },
            TranscriptEntry {
                role: "Planning".into(),
                message: "Delegate plan refinement: generalize instruction discovery\nSub-agent summary: reuse the workspace traversal helper".into(),
            },
            TranscriptEntry {
                role: "Agent".into(),
                message: "Here is the final recommendation.".into(),
            },
        ];

        let rendered = CommittedTurnCell::new(entries.as_slice(), Some(Path::new(".")))
            .display_lines(100)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        let you_idx = rendered.find("You: Review the workspace logic").unwrap();
        let explored_idx = rendered.find(" Explored ").unwrap();
        let planning_idx = rendered.find(" Planned ").unwrap();
        let agent_idx = rendered
            .find("Agent\n  Here is the final recommendation.")
            .unwrap();

        assert!(you_idx < explored_idx);
        assert!(explored_idx < planning_idx);
        assert!(planning_idx < agent_idx);
        assert!(rendered.contains("Sub-agent summary: current discovery is hardcoded"));
        assert!(rendered.contains("Sub-agent summary: reuse the workspace traversal helper"));
    }

    #[test]
    fn committed_turn_cell_places_completion_records_before_final_agent_message() {
        let entries = vec![
            TranscriptEntry {
                role: "You".into(),
                message: "Inspect the repo and decide whether to run the migration".into(),
            },
            TranscriptEntry {
                role: "Exploring".into(),
                message: "└ Read crates/instructions/src/workspace.rs".into(),
            },
            TranscriptEntry {
                role: "Shell Approval Completed".into(),
                message: "Bash approval: Approved once for command: bash ./scripts/migrate.sh"
                    .into(),
            },
            TranscriptEntry {
                role: "Agent".into(),
                message: "I approved the one-off shell step and can now continue with the final recommendation.".into(),
            },
        ];

        let rendered = CommittedTurnCell::new(entries.as_slice(), Some(Path::new(".")))
            .display_lines(100)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        let explored_idx = rendered.find(" Explored ").unwrap();
        let approval_idx = rendered.find(" Shell Approval Completed ").unwrap();
        let agent_idx = rendered
            .find("Agent\n  I approved the one-off shell step")
            .unwrap();

        assert!(explored_idx < approval_idx);
        assert!(approval_idx < agent_idx);
    }

    #[test]
    fn committed_turn_cell_orders_completion_records_by_interaction_kind() {
        let entries = vec![
            TranscriptEntry {
                role: "You".into(),
                message: "Inspect the workflow and capture the decision trail".into(),
            },
            TranscriptEntry {
                role: "Question Answered".into(),
                message: "Captured the generic answer.".into(),
            },
            TranscriptEntry {
                role: "Plan Decision".into(),
                message: "Approved the proposed implementation plan.".into(),
            },
            TranscriptEntry {
                role: "Shell Approval Completed".into(),
                message: "Approved the one-off shell command.".into(),
            },
            TranscriptEntry {
                role: "Planning Question Answered".into(),
                message: "Chose the plan_agent option.".into(),
            },
            TranscriptEntry {
                role: "Agent".into(),
                message: "Here is the final narrative summary.".into(),
            },
        ];

        let rendered = CommittedTurnCell::new(entries.as_slice(), Some(Path::new(".")))
            .display_lines(100)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        let shell_idx = rendered.find(" Shell Approval Completed ").unwrap();
        let plan_idx = rendered.find(" Plan Decision ").unwrap();
        let planning_question_idx = rendered.find(" Planning Question Answered ").unwrap();
        let generic_question_idx = rendered.find(" Question Answered ").unwrap();
        let agent_idx = rendered
            .find("Agent\n  Here is the final narrative summary.")
            .unwrap();

        assert!(shell_idx < plan_idx);
        assert!(plan_idx < planning_question_idx);
        assert!(planning_question_idx < generic_question_idx);
        assert!(generic_question_idx < agent_idx);
    }

    #[test]
    fn active_turn_cell_renders_plan_approval_as_interaction_card() {
        let temp = tempdir().unwrap();
        let mut app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("build tui app");
        app.active_turn = TranscriptTurn {
            entries: vec![TranscriptEntry {
                role: "You".into(),
                message: "Review the codebase and propose changes".into(),
            }],
        };
        app.snapshot.plan_steps = vec![
            ("pending".into(), "Generalize instruction discovery".into()),
            (
                "pending".into(),
                "Preserve cache and path resolution behavior".into(),
            ),
        ];
        app.snapshot.plan_explanation =
            Some("The current discovery path is hardcoded and should be generalized.".into());
        app.set_pending_plan_approval(true);

        let rendered = ActiveTurnCell::new(&app, Some(Path::new(".")))
            .display_lines(100)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains(" Awaiting Approval "));
        assert!(rendered.contains("Updated Plan"));
        assert!(rendered.contains("Start implementation now"));
        assert!(rendered.contains("Continue planning"));
        assert!(rendered.contains("Generalize instruction discovery"));
    }

    #[test]
    fn active_turn_cell_renders_updated_plan_checklist() {
        let temp = tempdir().unwrap();
        let mut app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("build tui app");
        app.active_turn = TranscriptTurn {
            entries: vec![TranscriptEntry {
                role: "You".into(),
                message: "Improve the plan rendering".into(),
            }],
        };
        app.snapshot.plan_steps = vec![
            ("completed".into(), "Inspect the current plan UI".into()),
            (
                "in_progress".into(),
                "Introduce a dedicated plan formatter".into(),
            ),
            (
                "pending".into(),
                "Unify status and transcript rendering".into(),
            ),
        ];
        app.snapshot.plan_explanation =
            Some("Keep the plan display aligned with Codex checklist semantics.".into());

        let rendered = ActiveTurnCell::new(&app, Some(Path::new(".")))
            .display_lines(100)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("Updated Plan"));
        assert!(rendered.contains("Keep the plan display aligned with Codex checklist semantics."));
        assert!(rendered.contains("✔ Inspect the current plan UI"));
        assert!(rendered.contains("□ Introduce a dedicated plan formatter"));
        assert!(rendered.contains("□ Unify status and transcript rendering"));
    }

    #[test]
    fn active_turn_cell_renders_shell_approval_as_interaction_card() {
        let temp = tempdir().unwrap();
        let mut app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("build tui app");
        app.active_turn = TranscriptTurn {
            entries: vec![TranscriptEntry {
                role: "You".into(),
                message: "Run the migration helper".into(),
            }],
        };
        app.snapshot
            .pending_interactions
            .push(crate::tui::state::PendingInteractionSnapshot {
                kind: crate::tui::state::InteractionKind::Approval,
                title: "Pending Approval".into(),
                summary: "bash ./scripts/migrate.sh".into(),
                options: Vec::new(),
                note: None,
                approval: Some(crate::tui::state::PendingApprovalSnapshot {
                    tool_use_id: "toolu_123".into(),
                    command: "bash ./scripts/migrate.sh".into(),
                    allow_net: false,
                    payload: crate::tools::bash::BashCommandInput {
                        command: None,
                        program: Some("bash".into()),
                        args: vec!["./scripts/migrate.sh".into()],
                        cwd: Some("/repo".into()),
                        env: Default::default(),
                        allow_net: false,
                    },
                }),
                source: None,
            });

        let rendered = ActiveTurnCell::new(&app, Some(Path::new(".")))
            .display_lines(100)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains(" Shell Approval "));
        assert!(rendered.contains("command:"));
        assert!(rendered.contains("cwd:"));
        assert!(rendered.contains("bash ./scripts/migrate.sh"));
    }

    #[test]
    fn active_turn_cell_renders_completed_plan_decision() {
        let temp = tempdir().unwrap();
        let mut app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("build tui app");
        app.active_turn = TranscriptTurn {
            entries: vec![TranscriptEntry {
                role: "You".into(),
                message: "Review the codebase and propose changes".into(),
            }],
        };
        app.record_completed_interaction(
            crate::tui::state::InteractionKind::PlanApproval,
            "Plan Decision",
            "Approved and started implementation",
            None,
        );

        let rendered = ActiveTurnCell::new(&app, Some(Path::new(".")))
            .display_lines(100)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains(" Plan Decision "));
        assert!(rendered.contains("Approved and started implementation"));
    }

    #[test]
    fn active_turn_cell_labels_delegated_plan_questions() {
        let temp = tempdir().unwrap();
        let mut app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("build tui app");
        app.active_turn = TranscriptTurn {
            entries: vec![TranscriptEntry {
                role: "You".into(),
                message: "Review the codebase and propose changes".into(),
            }],
        };
        app.record_local_request_input(
            "plan_agent",
            "Which discovery strategy should we keep?",
            vec![
                ("Minimal".into(), "Keep root-only files.".into()),
                (
                    "Generic".into(),
                    "Scan all instruction markdown files.".into(),
                ),
            ],
            Some("A product decision is needed before editing.".into()),
        );

        let rendered = ActiveTurnCell::new(&app, Some(Path::new(".")))
            .display_lines(100)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains(" Planning Question "));
        assert!(rendered.contains("source:"));
        assert!(rendered.contains("plan_agent"));
        assert!(rendered.contains("Which discovery strategy should we keep?"));
    }

    #[test]
    fn active_turn_cell_labels_delegated_completed_questions() {
        let temp = tempdir().unwrap();
        let mut app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("build tui app");
        app.active_turn = TranscriptTurn {
            entries: vec![TranscriptEntry {
                role: "You".into(),
                message: "Review the codebase and propose changes".into(),
            }],
        };
        app.record_completed_interaction(
            crate::tui::state::InteractionKind::RequestInput,
            "Which discovery strategy should we keep?",
            "Answered with: Generic",
            Some("plan_agent".into()),
        );

        let rendered = ActiveTurnCell::new(&app, Some(Path::new(".")))
            .display_lines(100)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains(" Planning Question Answered "));
        assert!(rendered.contains("Answered with: Generic"));
    }
}
