use std::path::Path;

use ratatui::text::Line;
use tempfile::tempdir;

use crate::config::ConfigManager;
use crate::tui::state::{TranscriptEntry, TranscriptTurn, TuiApp};

use super::cells::HistoryCell;
use super::{
    committed_turn_cell, current_turn_exploration_summary_from_entries, current_turn_tool_summary,
    desired_viewport_height, renderable_transcript_lines, transcript_scroll_offset,
    transcript_visual_row_count,
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

    assert!(rendered.contains("You: Earlier prompt"));
    assert!(rendered.contains("Committed answer"));
    assert!(rendered.contains("You: Current prompt"));
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
    assert_eq!(transcript_scroll_offset(&app, 3, visual_rows), visual_rows as u16 - 3);
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

    let rendered =
        current_turn_exploration_summary_from_entries(refs.as_slice(), false, None)
            .expect("exploration summary");
    assert!(rendered.contains("Read src/main.rs"));
    assert!(!rendered.contains("List ."));
    assert!(!rendered.contains("Glob src/**/*.rs"));
    assert!(!rendered.contains("Search planning mode src"));
    assert!(!rendered.contains("listing files"));
}
