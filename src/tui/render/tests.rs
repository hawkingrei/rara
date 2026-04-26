use std::path::Path;

use insta::assert_snapshot;
use ratatui::text::Line;
use ratatui::{buffer::Buffer, layout::Rect};
use tempfile::tempdir;

use crate::config::{ConfigManager, RaraConfig};
use crate::tui::custom_terminal::Frame;
use crate::tui::state::{Overlay, ProviderFamily, TranscriptEntry, TranscriptTurn, TuiApp};

use super::cells::HistoryCell;
use super::viewport::TranscriptViewport;
use super::{
    committed_turn_cell, compact_progress_summary_lines, compact_recent_first_summary_lines,
    compact_summary_text, current_turn_exploration_summary_from_entries, current_turn_tool_summary,
    desired_bottom_pane_height, desired_viewport_height, formatted_message_lines,
    prefixed_message_lines, renderable_transcript_lines, transcript_scroll_offset,
    transcript_viewport, transcript_visual_row_count,
};

#[test]
fn committed_turn_does_not_truncate_agent_response() {
    let entries = vec![
        TranscriptEntry {
            role: "You".into(),
            message: "Review the code".into(),
        },
        TranscriptEntry {
            role: "Agent".into(),
            message: (1..=12)
                .map(|idx| format!("Line {idx}"))
                .collect::<Vec<_>>()
                .join("\n"),
        },
    ];

    let rendered = committed_turn_cell(entries.as_slice(), Some(Path::new(".")))
        .display_lines(100)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("Line 12"));
    assert!(!rendered.contains("more line(s)"));
}

#[test]
fn keeps_history_reserve_once_transcript_exists() {
    let temp = tempdir().expect("tempdir");
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    app.committed_turns.push(TranscriptTurn {
        entries: vec![TranscriptEntry {
            role: "You".into(),
            message: "Earlier prompt".into(),
        }],
    });

    let height = desired_viewport_height(&app, 120, 24);
    assert!(height > 5);
    assert!(height < 24);
}

#[test]
fn tool_summary_includes_apply_patch_target_files() {
    let entries = vec![TranscriptEntry {
        role: "Tool".into(),
        message: "apply_patch src/tui/render.rs, src/tui/runtime/events.rs".into(),
    }];
    let refs = entries.iter().collect::<Vec<_>>();

    let rendered = current_turn_tool_summary(&refs, false, None).expect("tool summary");
    assert!(rendered.contains("Apply patch src/tui/render.rs, src/tui/runtime/events.rs"));
}

#[test]
fn renderable_transcript_lines_include_committed_and_active_turns() {
    let temp = tempdir().expect("tempdir");
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    app.committed_turns.push(TranscriptTurn {
        entries: vec![
            TranscriptEntry {
                role: "You".into(),
                message: "Earlier prompt".into(),
            },
            TranscriptEntry {
                role: "Agent".into(),
                message: "Committed answer".into(),
            },
        ],
    });
    app.active_turn.entries.push(TranscriptEntry {
        role: "You".into(),
        message: "Current prompt".into(),
    });

    let rendered = renderable_transcript_lines(&app, 100)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("› Earlier prompt"));
    assert!(rendered.contains("Committed answer"));
    assert!(rendered.contains("› Current prompt"));
}

#[test]
fn renderable_transcript_lines_insert_turn_dividers_between_rounds() {
    let temp = tempdir().expect("tempdir");
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    app.committed_turns = vec![
        TranscriptTurn {
            entries: vec![TranscriptEntry {
                role: "You".into(),
                message: "First prompt".into(),
            }],
        },
        TranscriptTurn {
            entries: vec![TranscriptEntry {
                role: "Agent".into(),
                message: "Second reply".into(),
            }],
        },
    ];
    app.active_turn.entries.push(TranscriptEntry {
        role: "You".into(),
        message: "Current prompt".into(),
    });

    let rendered = renderable_transcript_lines(&app, 24)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>();

    let divider = "─".repeat(24);
    assert_eq!(
        rendered
            .iter()
            .filter(|line| line.as_str() == divider)
            .count(),
        2
    );
}

#[test]
fn transcript_scroll_offset_keeps_zero_sticky_to_bottom() {
    let temp = tempdir().expect("tempdir");
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    app.transcript_scroll = 0;

    assert_eq!(transcript_scroll_offset(&app, 3, 10), 7);

    app.scroll_transcript(-2);
    assert_eq!(transcript_scroll_offset(&app, 3, 10), 5);
}

#[test]
fn transcript_scroll_offset_uses_wrapped_visual_height() {
    let temp = tempdir().expect("tempdir");
    let app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    let lines = vec![
        Line::from("Agent"),
        Line::from("  This is a long streamed response that should wrap across rows."),
    ];

    let visual_rows = transcript_visual_row_count(&lines, 12);
    assert!(visual_rows > lines.len());
    assert_eq!(
        transcript_scroll_offset(&app, 3, visual_rows),
        visual_rows as u16 - 3
    );
}

#[test]
fn renderable_transcript_lines_cache_is_invalidated_when_committed_turns_change() {
    let temp = tempdir().expect("tempdir");
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    app.restore_committed_turns(vec![TranscriptTurn {
        entries: vec![TranscriptEntry {
            role: "Agent".into(),
            message: "First answer".into(),
        }],
    }]);

    let first = renderable_transcript_lines(&app, 100)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(first.contains("First answer"));

    app.restore_committed_turns(vec![TranscriptTurn {
        entries: vec![TranscriptEntry {
            role: "Agent".into(),
            message: "Second answer".into(),
        }],
    }]);

    let second = renderable_transcript_lines(&app, 100)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!second.contains("First answer"));
    assert!(second.contains("Second answer"));
}

#[test]
fn transcript_viewport_is_independent_from_overlay_state() {
    let temp = tempdir().expect("tempdir");
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    app.committed_turns.push(TranscriptTurn {
        entries: vec![
            TranscriptEntry {
                role: "You".into(),
                message: "Earlier prompt".into(),
            },
            TranscriptEntry {
                role: "Agent".into(),
                message: "Committed answer".into(),
            },
        ],
    });
    app.active_turn.entries.push(TranscriptEntry {
        role: "You".into(),
        message: "Current prompt".into(),
    });

    let base = transcript_viewport(&app, 80, 18);
    app.overlay = Some(Overlay::Status);
    let with_overlay = transcript_viewport(&app, 80, 18);

    let base_rendered = base
        .lines
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>();
    let overlay_rendered = with_overlay
        .lines
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>();

    assert_eq!(base_rendered, overlay_rendered);
    assert_eq!(base.scroll_offset, with_overlay.scroll_offset);
}

#[test]
fn transcript_viewport_keeps_manual_scroll_when_overlay_opens() {
    let temp = tempdir().expect("tempdir");
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    app.committed_turns.push(TranscriptTurn {
        entries: vec![
            TranscriptEntry {
                role: "You".into(),
                message: "Earlier prompt".into(),
            },
            TranscriptEntry {
                role: "Agent".into(),
                message: (1..=8)
                    .map(|idx| format!("Line {idx}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            },
        ],
    });
    app.scroll_transcript(-3);

    let base = transcript_viewport(&app, 60, 8);
    app.overlay = Some(Overlay::Status);
    let with_overlay = transcript_viewport(&app, 60, 8);

    assert_eq!(base.scroll_offset, with_overlay.scroll_offset);
    assert_eq!(app.transcript_scroll, 3);
}

#[test]
fn command_palette_does_not_change_scrolled_viewport_height() {
    let temp = tempdir().expect("tempdir");
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    app.transcript_scroll = 5;

    let base = desired_viewport_height(&app, 80, 24);
    app.overlay = Some(Overlay::CommandPalette);
    let with_palette = desired_viewport_height(&app, 80, 24);

    assert_eq!(base, 24);
    assert_eq!(base, with_palette);
}

#[test]
fn bottom_pane_grows_for_multiline_input() {
    let temp = tempdir().expect("tempdir");
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");

    let base = desired_bottom_pane_height(&app, 80, 24);
    app.input = "first line\nsecond line\nthird line\nfourth line".into();
    let expanded = desired_bottom_pane_height(&app, 80, 24);

    assert_eq!(base, 5);
    assert!(expanded > base);
}

#[test]
fn bottom_pane_preserves_space_only_input_layout() {
    let temp = tempdir().expect("tempdir");
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");

    app.input = " ".into();
    let space_only = desired_bottom_pane_height(&app, 80, 24);

    app.input = "  \n ".into();
    let multiline_space_only = desired_bottom_pane_height(&app, 80, 24);

    assert_eq!(space_only, 5);
    assert!(multiline_space_only >= space_only);
}

#[test]
fn bottom_pane_height_does_not_panic_on_tiny_terminal() {
    let temp = tempdir().expect("tempdir");
    let app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");

    assert_eq!(desired_bottom_pane_height(&app, 80, 1), 1);
    assert_eq!(desired_bottom_pane_height(&app, 80, 3), 3);
}

#[test]
fn transcript_viewport_visible_window_keeps_partial_wrapped_line_offset() {
    let viewport = TranscriptViewport::new(
        vec![
            Line::from("• This is a long first line that wraps across rows."),
            Line::from("  Second line stays visible."),
        ],
        1,
    );

    let (lines, inner_scroll) = viewport.visible_window(12, 3);
    let rendered = lines
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>();

    assert_eq!(inner_scroll, 1);
    assert_eq!(rendered.len(), 1);
    assert!(rendered[0].contains("long first line"));
}

#[test]
fn transcript_viewport_visible_window_slices_to_visible_rows() {
    let viewport = TranscriptViewport::new(
        vec![
            Line::from("› First"),
            Line::from("• Second"),
            Line::from("  Third"),
            Line::from("  Fourth"),
        ],
        1,
    );

    let (lines, inner_scroll) = viewport.visible_window(80, 2);
    let rendered = lines
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>();

    assert_eq!(inner_scroll, 0);
    assert_eq!(rendered, vec!["• Second", "  Third"]);
}

#[test]
fn exploration_summary_only_keeps_read_actions() {
    let entries = vec![
        TranscriptEntry {
            role: "Tool".into(),
            message: "list_files .".into(),
        },
        TranscriptEntry {
            role: "Tool".into(),
            message: "glob src/**/*.rs".into(),
        },
        TranscriptEntry {
            role: "Tool".into(),
            message: "grep planning mode src".into(),
        },
        TranscriptEntry {
            role: "Tool".into(),
            message: "read_file src/main.rs".into(),
        },
        TranscriptEntry {
            role: "Agent".into(),
            message: "I will start by listing files and then inspect the main entrypoint.".into(),
        },
    ];
    let refs = entries.iter().collect::<Vec<_>>();

    let rendered = current_turn_exploration_summary_from_entries(refs.as_slice(), false, None)
        .expect("exploration summary");
    assert!(rendered.contains("Read src/main.rs"));
    assert!(!rendered.contains("List ."));
    assert!(!rendered.contains("Glob src/**/*.rs"));
    assert!(!rendered.contains("Search planning mode src"));
    assert!(!rendered.contains("listing files"));
}

#[test]
fn compact_progress_summary_lines_prioritizes_latest_note_and_recent_actions() {
    let actions = vec![
        "Read src/module_1.rs".to_string(),
        "Read src/module_2.rs".to_string(),
        "Read src/module_3.rs".to_string(),
    ];
    let notes = vec![
        "Initial inspection complete.".to_string(),
        "Next I will verify the persistence path.".to_string(),
    ];

    let rendered = compact_progress_summary_lines(
        actions.as_slice(),
        notes.as_slice(),
        2,
        "more exploration step(s)",
    );

    assert!(rendered.contains("Next I will verify the persistence path."));
    assert!(!rendered.contains("Initial inspection complete."));
    assert!(rendered.contains("... 1 more exploration step(s)"));
    assert!(rendered.contains("Read src/module_2.rs"));
    assert!(rendered.contains("Read src/module_3.rs"));
}

#[test]
fn compact_recent_first_summary_lines_puts_current_running_step_first() {
    let items = vec![
        "Run task 1".to_string(),
        "Run task 2".to_string(),
        "Run task 3".to_string(),
        "Run task 4".to_string(),
        "Run task 5".to_string(),
    ];

    let rendered = compact_recent_first_summary_lines(items.as_slice(), 4, "more running step(s)");

    let lines = rendered.lines().collect::<Vec<_>>();
    assert_eq!(lines[0], "└ Run task 5");
    assert_eq!(lines[1], "└ ... 1 more running step(s)");
    assert!(rendered.contains("Run task 4"));
    assert!(rendered.contains("Run task 2"));
    assert!(!rendered.contains("Run task 1"));
}

#[test]
fn exploration_summary_compacts_long_read_lists() {
    let entries = (1..=6)
        .map(|idx| TranscriptEntry {
            role: "Tool".into(),
            message: format!("read_file src/module_{idx}.rs"),
        })
        .collect::<Vec<_>>();
    let refs = entries.iter().collect::<Vec<_>>();

    let rendered = current_turn_exploration_summary_from_entries(refs.as_slice(), false, None)
        .expect("exploration summary");
    assert!(rendered.contains("... 2 more file(s) inspected"));
    assert!(!rendered.contains("module_1.rs"));
    assert!(!rendered.contains("module_2.rs"));
    assert!(rendered.contains("module_3.rs"));
    assert!(rendered.contains("module_6.rs"));
}

#[test]
fn compact_summary_text_keeps_tail_of_long_explicit_blocks() {
    let summary = [
        "└ Read src/a.rs",
        "└ Read src/b.rs",
        "└ Read src/c.rs",
        "└ Read src/d.rs",
        "└ Read src/e.rs",
    ]
    .join("\n");

    let rendered = compact_summary_text(&summary, 4, "more exploration step(s)");
    assert!(rendered.contains("... 1 more exploration step(s)"));
    assert!(!rendered.contains("src/a.rs"));
    assert!(rendered.contains("src/b.rs"));
    assert!(rendered.contains("src/e.rs"));
}

#[test]
fn ssh_startup_page_warns_without_opening_setup_window() {
    let temp = tempdir().expect("tempdir");
    let old_ssh_connection = std::env::var_os("SSH_CONNECTION");
    let old_ssh_tty = std::env::var_os("SSH_TTY");
    std::env::set_var("SSH_CONNECTION", "test");

    let cm = ConfigManager {
        path: temp.path().join("config.json"),
    };
    let mut config = RaraConfig::default();
    config.set_provider("openai-compatible");
    config.clear_api_key();
    cm.save(&config).expect("save config");

    let app = TuiApp::new(cm).expect("build tui app");
    assert!(app.overlay.is_none());

    let rendered = render_screen_text(&app, 100, 24);
    assert_snapshot!("ssh_startup_warning_screen", rendered);

    if let Some(value) = old_ssh_connection {
        std::env::set_var("SSH_CONNECTION", value);
    } else {
        std::env::remove_var("SSH_CONNECTION");
    }
    if let Some(value) = old_ssh_tty {
        std::env::set_var("SSH_TTY", value);
    } else {
        std::env::remove_var("SSH_TTY");
    }
}

#[test]
fn provider_picker_renders_as_full_overlay_on_standard_terminal() {
    let temp = tempdir().expect("tempdir");
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    app.open_overlay(Overlay::ProviderPicker);

    let rendered = render_screen_text(&app, 100, 24);
    assert_snapshot!("provider_picker_standard_terminal", rendered);
}

#[test]
fn provider_picker_does_not_panic_on_106x25_terminal_after_model_command() {
    let temp = tempdir().expect("tempdir");
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    app.input = "/model".into();
    app.open_overlay(Overlay::ProviderPicker);

    let rendered = render_screen_text(&app, 106, 25);
    assert!(rendered.contains("Provider Menu"));
    assert!(rendered.contains("OpenAI-compatible"));
}

#[test]
fn command_palette_does_not_panic_on_107x53_terminal_with_slash_input() {
    let temp = tempdir().expect("tempdir");
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    app.input = "/stat".into();
    app.open_overlay(Overlay::CommandPalette);

    let rendered = render_screen_text(&app, 107, 53);
    assert!(rendered.contains("/status"));
}

#[test]
fn command_palette_query_uses_full_width_without_leaking_bottom_status() {
    let temp = tempdir().expect("tempdir");
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    app.input = "/m".into();
    app.open_overlay(Overlay::CommandPalette);

    let rendered = render_screen_text(&app, 107, 53);
    assert!(rendered.contains("/model"));
    assert!(!rendered.contains("ctx~=0"));
    assert!(!rendered.contains("enter run  esc close"));
}

#[test]
fn command_palette_empty_query_does_not_render_inline_footer_hint() {
    let temp = tempdir().expect("tempdir");
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    app.input = "/".into();
    app.open_overlay(Overlay::CommandPalette);

    let rendered = render_screen_text(&app, 107, 53);
    assert!(rendered.contains("/approval"));
    assert!(rendered.contains("/model"));
    assert!(!rendered.contains("enter run  esc close"));
    assert!(!rendered.contains("up/down move  enter run  esc close"));
}

#[test]
fn provider_picker_does_not_panic_on_107x53_terminal_after_model_command() {
    let temp = tempdir().expect("tempdir");
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    app.input = "/model".into();
    app.open_overlay(Overlay::ProviderPicker);

    let rendered = render_screen_text(&app, 107, 53);
    assert!(rendered.contains("Provider Menu"));
}

#[test]
fn auth_mode_picker_does_not_panic_on_107x53_terminal_after_auth_command() {
    let temp = tempdir().expect("tempdir");
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    app.input = "/auth".into();
    app.open_overlay(Overlay::AuthModePicker);

    let rendered = render_screen_text(&app, 107, 53);
    assert!(rendered.contains("Codex Login"));
}

#[test]
fn render_clamps_cursor_to_frame_bounds() {
    let temp = tempdir().expect("tempdir");
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    app.input = "/model".into();
    app.open_overlay(Overlay::ProviderPicker);

    let area = Rect::new(0, 0, 106, 25);
    let mut buffer = Buffer::empty(area);
    let mut frame = Frame {
        cursor_position: None,
        viewport_area: area,
        buffer: &mut buffer,
    };
    super::render(&mut frame, &app);

    let cursor = frame.cursor_position.expect("cursor should be set");
    assert!(cursor.x < area.right());
    assert!(cursor.y < area.bottom());
}

#[test]
fn api_key_editor_renders_full_prompt_on_standard_terminal() {
    let temp = tempdir().expect("tempdir");
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    let mut config = RaraConfig::default();
    config.set_provider("openai-compatible");
    config.base_url = Some("https://api.deepseek.com".into());
    config.model = Some("deepseek-chat".into());
    app.config = config;
    app.provider_picker_idx = crate::tui::state::PROVIDER_FAMILIES
        .iter()
        .position(|(family, _, _)| *family == ProviderFamily::OpenAiCompatible)
        .expect("openai-compatible family present");
    app.open_overlay(Overlay::ApiKeyEditor);

    let rendered = render_screen_text(&app, 100, 24);
    assert_snapshot!("api_key_editor_standard_terminal", rendered);
}

fn render_screen_text(app: &TuiApp, width: u16, height: u16) -> String {
    let area = Rect::new(0, 0, width, height);
    let mut buffer = Buffer::empty(area);
    let mut frame = Frame {
        cursor_position: None,
        viewport_area: area,
        buffer: &mut buffer,
    };
    super::render(&mut frame, app);

    (0..height)
        .map(|y| {
            let mut line = String::new();
            for x in 0..width {
                line.push_str(buffer[(x, y)].symbol());
            }
            line.trim_end().to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn prefixed_message_lines_keep_first_and_latest_lines() {
    let rendered = prefixed_message_lines(
        "Agent",
        &["intro", "middle 1", "middle 2", "latest 1", "latest 2"].join("\n"),
        3,
    )
    .into_iter()
    .map(|line| line.to_string())
    .collect::<Vec<_>>();

    assert_eq!(rendered[0], "Agent: intro");
    assert_eq!(rendered[1], "  ... 2 more line(s)");
    assert_eq!(rendered[2], "  latest 1");
    assert_eq!(rendered[3], "  latest 2");
}

#[test]
fn prefixed_message_lines_show_truncation_when_max_lines_is_one() {
    let agent_rendered = prefixed_message_lines("Agent", &["intro", "latest 1"].join("\n"), 1)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>();
    assert_eq!(agent_rendered[0], "Agent: intro");
    assert_eq!(agent_rendered[1], "  ... 1 more line(s)");

    let user_rendered = prefixed_message_lines("You", &["intro", "latest 1"].join("\n"), 1)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>();
    assert_eq!(user_rendered[0], "› intro");
    assert_eq!(user_rendered[1], "  ... 1 more line(s)");
}

#[test]
fn formatted_agent_markdown_keeps_first_and_latest_lines() {
    let rendered = formatted_message_lines(
        "Agent",
        &["first line", "middle 1", "middle 2", "latest 1", "latest 2"].join("\n"),
        3,
        Some(Path::new(".")),
    )
    .into_iter()
    .map(|line| line.to_string())
    .collect::<Vec<_>>();

    assert!(rendered.iter().any(|line| line.contains("first line")));
    assert!(rendered
        .iter()
        .any(|line| line.contains("... 2 more line(s)")));
    assert!(rendered.iter().any(|line| line.contains("latest 1")));
    assert!(rendered.iter().any(|line| line.contains("latest 2")));
    assert!(!rendered.iter().any(|line| line.contains("middle 1")));
}
