// Claude-style /status display — clean, minimal, sectioned layout.
//
// Replaces the dense key=value grid with semantically grouped
// sections that use visual hierarchy and color for emphasis.
use crate::tui::theme::*;
use ratatui::style::Color;

use crate::tui::state::TuiApp;

pub(crate) fn render_status_lines(app: &TuiApp) -> Vec<String> {
    let mut lines = Vec::new();

    // ── Provider & Model ──
    section_header(&mut lines, "Provider & Model");
    kv(&mut lines, "provider", &app.config.provider, TEXT_ACCENT);
    kv(&mut lines, "model", app.current_model_label(), STATUS_INFO);
    if app.config.provider == "openai-compatible" {
        kv(
            &mut lines,
            "endpoint",
            app.config.active_openai_profile_label().unwrap_or("-"),
            TEXT_SECONDARY,
        );
    }

    // ── Execution ──
    section_spacer(&mut lines);
    section_header(&mut lines, "Execution");
    kv(
        &mut lines,
        "mode",
        app.agent_execution_mode_label(),
        STATUS_INFO,
    );
    kv(
        &mut lines,
        "phase",
        app.runtime_phase_label(),
        TEXT_SECONDARY,
    );
    if let Some(detail) = &app.runtime_phase_detail {
        kv(&mut lines, "detail", detail, TEXT_MUTED);
    }
    kv(
        &mut lines,
        "bash",
        app.bash_approval_mode_label(),
        TEXT_SECONDARY,
    );

    // ── Context ──
    section_spacer(&mut lines);
    section_header(&mut lines, "Context");
    let snap = &app.snapshot;
    kv(
        &mut lines,
        "history",
        &format!("{} tokens", snap.estimated_history_tokens),
        TEXT_SECONDARY,
    );
    if let Some(window) = snap.context_window_tokens {
        kv(
            &mut lines,
            "window",
            &format_metric(window as u64),
            STATUS_INFO,
        );
    }
    if let Some(remaining) = snap.remaining_input_budget {
        kv(
            &mut lines,
            "budget",
            &format!("{} tokens remaining", remaining),
            if remaining < 1024 {
                STATUS_WARNING
            } else {
                TEXT_SECONDARY
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
            STATUS_SUCCESS,
        );
    }
    if snap.compaction_count > 0 {
        kv(
            &mut lines,
            "compactions",
            &snap.compaction_count.to_string(),
            TEXT_SECONDARY,
        );
    }

    // ── Workspace ──
    section_spacer(&mut lines);
    section_header(&mut lines, "Workspace");
    kv_short(&mut lines, "dir", &home_path(&snap.cwd), TEXT_SECONDARY);
    kv_short(&mut lines, "branch", &snap.branch, TEXT_SECONDARY);
    kv_short(&mut lines, "session", &snap.session_id, TEXT_MUTED);

    // ── API & Auth ──
    section_spacer(&mut lines);
    section_header(&mut lines, "API & Auth");
    let surface = app.config.effective_provider_surface();
    kv_short(
        &mut lines,
        "base_url",
        surface.base_url.display_or("-"),
        TEXT_SECONDARY,
    );
    kv_short(
        &mut lines,
        "api_key",
        &api_key_label(app),
        if app.config.has_api_key() {
            STATUS_SUCCESS
        } else {
            STATUS_WARNING
        },
    );
    kv_short(
        &mut lines,
        "reasoning",
        &format!(
            "{} ({})",
            surface.reasoning_summary.display_or("auto"),
            surface.reasoning_summary.source.label()
        ),
        TEXT_SECONDARY,
    );

    // ── Network & Sandbox ──
    section_spacer(&mut lines);
    section_header(&mut lines, "Network & Sandbox");
    kv_short(&mut lines, "sandbox", &sandbox_label(app), STATUS_INFO);
    kv_short(
        &mut lines,
        "network",
        if app.config.sandbox_workspace_write.network_access {
            "permitted"
        } else {
            "restricted"
        },
        if app.config.sandbox_workspace_write.network_access {
            STATUS_WARNING
        } else {
            STATUS_SUCCESS
        },
    );

    lines
}

fn section_header(lines: &mut Vec<String>, title: &str) {
    lines.push(format!("│ {title}"));
    lines.push("│".to_string());
}

fn section_spacer(lines: &mut Vec<String>) {
    lines.push("│".to_string());
}

fn kv(lines: &mut Vec<String>, key: &str, value: &str, _color: Color) {
    lines.push(format!("│  {key:<14} {value}"));
}

fn kv_short(lines: &mut Vec<String>, key: &str, value: &str, _color: Color) {
    lines.push(format!("│  {key:<14} {value}"));
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

fn sandbox_label(app: &TuiApp) -> String {
    if cfg!(target_os = "macos") {
        "macos-seatbelt"
    } else if cfg!(target_os = "linux") {
        "linux-bubblewrap"
    } else {
        "none"
    }
    .to_string()
}
