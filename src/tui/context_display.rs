// Claude Code-style /context display — visual budget bar, clean sections, percentages.
//
// Each line is a ratatui Line with Span-styled values so colors
// actually render in the TUI, not just plain text.
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::tui::state::TuiApp;
use crate::tui::theme::{
    BUDGET_ACTIVE, BUDGET_FREE, BUDGET_HISTORY, BUDGET_MEMORY, BUDGET_OUTPUT, BUDGET_SYSTEM,
    BUDGET_WORKSPACE, STATUS_INFO, STATUS_SUCCESS, TEXT_ACCENT, TEXT_MUTED, TEXT_SECONDARY,
};

pub(crate) fn render_context_lines(app: &TuiApp, available_width: u16) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let snap = &app.snapshot;

    let window = snap.context_window_tokens;
    let used = snap
        .stable_instructions_budget
        .saturating_add(snap.workspace_prompt_budget)
        .saturating_add(snap.active_turn_budget)
        .saturating_add(snap.compacted_history_budget)
        .saturating_add(snap.retrieved_memory_budget)
        .saturating_add(snap.reserved_output_tokens);

    // ── Context Usage ──
    section_header(&mut lines, "Context Usage");
    kv(
        &mut lines,
        "model",
        app.current_model_label(),
        Color::LightBlue,
    );
    kv(
        &mut lines,
        "window",
        &format!(
            "{} tokens",
            window
                .map(format_token_count)
                .unwrap_or_else(|| "unknown".to_string())
        ),
        Color::DarkGray,
    );

    // Visual budget bar
    let bar_width = (available_width.saturating_sub(6)).max(20) as usize;
    let bar_line = budget_bar(app, bar_width);
    lines.push(bar_line);

    // Usage summary
    let used_str = format_token_count(used);
    let pct = window
        .filter(|w| *w > 0)
        .map(|w| format!(" ({:.2}%)", used as f64 * 100.0 / w as f64))
        .unwrap_or_default();
    let window_str = window
        .map(format_token_count)
        .unwrap_or_else(|| "?".to_string());
    lines.push(Line::from(Span::styled(
        format!("  {used_str}{pct} of {window_str} used"),
        Style::default().fg(TEXT_MUTED),
    )));
    section_spacer(&mut lines);

    // ── Budget Breakdown ──
    section_header(&mut lines, "Budget Breakdown");
    budget_row(
        &mut lines,
        "System prompt",
        snap.stable_instructions_budget,
        BUDGET_SYSTEM,
        window,
    );
    budget_row(
        &mut lines,
        "Workspace",
        snap.workspace_prompt_budget,
        BUDGET_WORKSPACE,
        window,
    );
    budget_row(
        &mut lines,
        "Active turn",
        snap.active_turn_budget,
        BUDGET_ACTIVE,
        window,
    );
    budget_row(
        &mut lines,
        "History",
        snap.compacted_history_budget,
        BUDGET_HISTORY,
        window,
    );
    budget_row(
        &mut lines,
        "Memory",
        snap.retrieved_memory_budget,
        BUDGET_MEMORY,
        window,
    );
    budget_row(
        &mut lines,
        "Output reserve",
        snap.reserved_output_tokens,
        BUDGET_OUTPUT,
        window,
    );
    if let Some(free) = snap.remaining_input_budget {
        budget_row(&mut lines, "Free", free, BUDGET_FREE, window);
    }
    section_spacer(&mut lines);

    // ── Session ──
    section_header(&mut lines, "Session");
    kv(&mut lines, "cwd", &home_path(&snap.cwd), Color::DarkGray);
    kv(&mut lines, "branch", &snap.branch, Color::DarkGray);
    kv(&mut lines, "session", &snap.session_id, Color::Gray);
    kv(
        &mut lines,
        "history",
        &format!(
            "{} msgs  {} entries",
            snap.history_len,
            app.transcript_entry_count()
        ),
        Color::DarkGray,
    );
    section_spacer(&mut lines);

    // ── Compaction ──
    if snap.compaction_count > 0 {
        section_header(&mut lines, "Compaction");
        kv(
            &mut lines,
            "estimated",
            &format!(
                "{} tokens",
                format_token_count(snap.estimated_history_tokens)
            ),
            Color::DarkGray,
        );
        kv(
            &mut lines,
            "threshold",
            &format!(
                "{} tokens",
                format_token_count(snap.compact_threshold_tokens)
            ),
            Color::DarkGray,
        );
        kv(
            &mut lines,
            "count",
            &snap.compaction_count.to_string(),
            Color::DarkGray,
        );
        if let (Some(before), Some(after)) = (
            snap.last_compaction_before_tokens,
            snap.last_compaction_after_tokens,
        ) {
            kv(
                &mut lines,
                "last",
                &format!(
                    "{} → {} tokens",
                    format_token_count(before),
                    format_token_count(after)
                ),
                Color::Gray,
            );
        }
        section_spacer(&mut lines);
    }

    // ── Active Turn ──
    section_header(&mut lines, "Active Turn");
    kv(
        &mut lines,
        "mode",
        app.agent_execution_mode_label(),
        Color::LightBlue,
    );
    if snap.plan_steps.is_empty() {
        kv(&mut lines, "plan", "no active plan steps", Color::Gray);
    } else {
        for (idx, (status, step)) in snap.plan_steps.iter().enumerate() {
            let color = match status.as_str() {
                "pending" => Color::DarkGray,
                "in_progress" => STATUS_INFO,
                "completed" => STATUS_SUCCESS,
                _ => Color::Gray,
            };
            kv(
                &mut lines,
                &format!("step {idx}"),
                &format!("[{status}] {step}"),
                color,
            );
        }
    }
    if !snap.pending_interactions.is_empty() {
        kv(
            &mut lines,
            "pending",
            &format!("{} interaction(s)", snap.pending_interactions.len()),
            Color::Yellow,
        );
    }

    lines
}

// ── budget bar ──

fn budget_bar(app: &TuiApp, width: usize) -> Line<'static> {
    let snap = &app.snapshot;
    let total = snap.context_window_tokens.unwrap_or(1).max(1);

    let segments: &[(usize, Color)] = &[
        (snap.stable_instructions_budget, BUDGET_SYSTEM),
        (snap.workspace_prompt_budget, BUDGET_WORKSPACE),
        (snap.active_turn_budget, BUDGET_ACTIVE),
        (snap.compacted_history_budget, BUDGET_HISTORY),
        (snap.retrieved_memory_budget, BUDGET_MEMORY),
        (snap.reserved_output_tokens, BUDGET_OUTPUT),
    ];

    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut used_width = 0usize;

    for &(tokens, color) in segments {
        let seg_width = ((tokens as f64 / total as f64) * width as f64).round() as usize;
        let seg_width = seg_width.min(width.saturating_sub(used_width));
        if seg_width > 0 {
            spans.push(Span::styled(
                "█".repeat(seg_width),
                Style::default().fg(color),
            ));
            used_width += seg_width;
        }
    }

    // Fill remaining with free color
    if used_width < width {
        spans.push(Span::styled(
            "█".repeat(width - used_width),
            Style::default().fg(BUDGET_FREE),
        ));
    }

    // Prefix with two spaces for alignment with kv lines
    spans.insert(0, Span::raw("  "));
    Line::from(spans)
}

fn budget_row(
    lines: &mut Vec<Line<'static>>,
    label: &str,
    tokens: usize,
    color: Color,
    window: Option<usize>,
) {
    let pct = window
        .filter(|w| *w > 0)
        .map(|w| format!(" ({:.2}%)", tokens as f64 * 100.0 / w as f64))
        .unwrap_or_default();
    let value = format!("{}{}", format_token_count(tokens), pct);
    kv(lines, label, &value, color);
}

// ── helpers ──

fn section_header(lines: &mut Vec<Line<'static>>, title: &str) {
    lines.push(Line::from(Span::styled(
        title.to_string(),
        Style::default()
            .fg(TEXT_ACCENT)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));
}

fn section_spacer(lines: &mut Vec<Line<'static>>) {
    lines.push(Line::from(""));
}

fn kv(lines: &mut Vec<Line<'static>>, key: &str, value: &str, value_color: Color) {
    let key_span = Span::styled(format!("  {key:<14} "), Style::default().fg(TEXT_SECONDARY));
    let value_span = Span::styled(value.to_string(), Style::default().fg(value_color));
    lines.push(Line::from(vec![key_span, value_span]));
}

fn format_token_count(tokens: usize) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.1}K", tokens as f64 / 1_000.0)
    } else {
        tokens.to_string()
    }
}

fn home_path(cwd: &str) -> String {
    if let Ok(home) = std::env::var("HOME") {
        if let Some(stripped) = cwd.strip_prefix(&home) {
            return format!("~{}", stripped);
        }
    }
    cwd.to_string()
}
