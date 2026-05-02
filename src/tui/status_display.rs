// Claude-style /status display — clean, sectioned, color-styled output.
//
// Each line is a ratatui Line with Span-styled values so colors
// actually render in the TUI, not just plain text.
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::tui::state::TuiApp;

pub(crate) fn render_status_lines(app: &TuiApp) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    // ── Provider & Model ──
    section_header(&mut lines, "Provider & Model");
    kv(&mut lines, "provider", &app.config.provider, Color::Cyan);
    kv(
        &mut lines,
        "model",
        app.current_model_label(),
        Color::LightBlue,
    );
    if app.config.provider == "openai-compatible" {
        kv(
            &mut lines,
            "endpoint",
            app.config.active_openai_profile_label().unwrap_or("-"),
            Color::DarkGray,
        );
    }

    // ── Execution ──
    section_spacer(&mut lines);
    section_header(&mut lines, "Execution");
    kv(
        &mut lines,
        "mode",
        app.agent_execution_mode_label(),
        Color::LightBlue,
    );
    kv(
        &mut lines,
        "phase",
        app.runtime_phase_label(),
        Color::DarkGray,
    );
    if let Some(detail) = &app.runtime_phase_detail {
        kv(&mut lines, "detail", detail, Color::Gray);
    }
    kv(
        &mut lines,
        "bash",
        app.bash_approval_mode_label(),
        Color::DarkGray,
    );

    // ── Context ──
    section_spacer(&mut lines);
    section_header(&mut lines, "Context");
    let snap = &app.snapshot;
    kv(
        &mut lines,
        "history",
        &format!("{} tokens", snap.estimated_history_tokens),
        Color::DarkGray,
    );
    if let Some(window) = snap.context_window_tokens {
        kv(
            &mut lines,
            "window",
            &format_metric(window as u64),
            Color::LightBlue,
        );
    }
    if let Some(remaining) = snap.remaining_input_budget {
        kv(
            &mut lines,
            "budget",
            &format!("{} tokens remaining", remaining),
            if remaining < 1024 {
                Color::Yellow
            } else {
                Color::DarkGray
            },
        );
    }
    if snap.total_input_tokens > 0 || snap.total_output_tokens > 0 {
        let hit = snap.total_cache_hit_tokens;
        let miss = snap.total_cache_miss_tokens;
        let total = hit + miss;
        let rate = if total > 0 {
            (hit as f64 / total as f64 * 100.0) as u32
        } else {
            0
        };
        kv(
            &mut lines,
            "cache",
            &format!("{}% hit ({} hits / {} misses)", rate, hit, miss),
            Color::LightGreen,
        );
    }
    if snap.compaction_count > 0 {
        kv(
            &mut lines,
            "compactions",
            &snap.compaction_count.to_string(),
            Color::DarkGray,
        );
    }

    // ── Workspace ──
    section_spacer(&mut lines);
    section_header(&mut lines, "Workspace");
    kv(&mut lines, "dir", &home_path(&snap.cwd), Color::DarkGray);
    kv(&mut lines, "branch", &snap.branch, Color::DarkGray);
    kv(&mut lines, "session", &snap.session_id, Color::Gray);

    // ── API & Auth ──
    section_spacer(&mut lines);
    section_header(&mut lines, "API & Auth");
    let surface = app.config.effective_provider_surface();
    kv(
        &mut lines,
        "base_url",
        surface.base_url.display_or("-"),
        Color::DarkGray,
    );
    kv(
        &mut lines,
        "api_key",
        &api_key_label(app),
        if app.config.has_api_key() {
            Color::LightGreen
        } else {
            Color::Yellow
        },
    );
    kv(
        &mut lines,
        "reasoning",
        &format!(
            "{} ({})",
            surface.reasoning_summary.display_or("auto"),
            surface.reasoning_summary.source.label()
        ),
        Color::DarkGray,
    );

    // ── Network & Sandbox ──
    section_spacer(&mut lines);
    section_header(&mut lines, "Network & Sandbox");
    kv(&mut lines, "sandbox", &sandbox_label(app), Color::LightBlue);
    kv(
        &mut lines,
        "network",
        if app.config.sandbox_workspace_write.network_access {
            "permitted"
        } else {
            "restricted"
        },
        if app.config.sandbox_workspace_write.network_access {
            Color::Yellow
        } else {
            Color::LightGreen
        },
    );

    lines
}

// ── helpers ──

fn section_header(lines: &mut Vec<Line<'static>>, title: &str) {
    lines.push(Line::from(Span::styled(
        title.to_string(),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));
}

fn section_spacer(lines: &mut Vec<Line<'static>>) {
    lines.push(Line::from(""));
}

fn kv(lines: &mut Vec<Line<'static>>, key: &str, value: &str, value_color: Color) {
    let key_span = Span::styled(
        format!("  {key:<14} "),
        Style::default().fg(Color::DarkGray),
    );
    let value_span = Span::styled(value.to_string(), Style::default().fg(value_color));
    lines.push(Line::from(vec![key_span, value_span]));
}

fn format_metric(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{}K", n / 1_000)
    } else {
        n.to_string()
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

fn api_key_label(app: &TuiApp) -> String {
    if app.config.has_api_key() {
        let source = app
            .config
            .effective_provider_surface()
            .api_key
            .source
            .label();
        format!("●●●●● ({source})")
    } else {
        "not set".to_string()
    }
}

fn sandbox_label(_app: &TuiApp) -> String {
    if cfg!(target_os = "macos") {
        "macos-seatbelt"
    } else if cfg!(target_os = "linux") {
        "linux-bubblewrap"
    } else {
        "none"
    }
    .to_string()
}
