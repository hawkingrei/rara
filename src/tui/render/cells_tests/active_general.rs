use super::*;

#[test]
fn active_turn_cell_keeps_sections_in_stable_order() {
    let temp = tempdir().unwrap();
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    app.agent_execution_mode = crate::agent::AgentExecutionMode::Plan;
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

    let you_idx = rendered.find("› Inspect the codebase").unwrap();
    let running_idx = rendered.find(" Running ").unwrap();
    let plan_idx = rendered.find("Updated Plan").unwrap();
    let approval_idx = rendered.find(" Request Input ").unwrap();

    assert!(!rendered.contains(" Exploring "));
    assert!(you_idx < running_idx);
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

    assert!(rendered.contains("› Review this repository and propose changes."));
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
    assert!(!rendered.contains("waiting for model response · 12s elapsed"));
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
    assert!(!rendered.contains("waiting for model response · 20s elapsed"));
}

#[test]
fn active_turn_cell_compacts_long_live_exploration_sections() {
    let temp = tempdir().unwrap();
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    app.runtime_phase = RuntimePhase::RunningTool;
    app.active_turn = TranscriptTurn {
        entries: vec![TranscriptEntry {
            role: "You".into(),
            message: "Inspect the repository".into(),
        }],
    };
    for idx in 1..=5 {
        app.record_exploration_action(format!("Read src/module_{idx}.rs"));
    }
    app.record_exploration_note("Cross-check the auth entrypoint.");

    let rendered = ActiveTurnCell::new(&app, Some(Path::new(".")))
        .display_lines(100)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains(" Exploring "));
    assert!(rendered.contains("... 2 more exploration step(s)"));
    assert!(!rendered.contains("module_1.rs"));
    assert!(!rendered.contains("module_2.rs"));
    assert!(rendered.contains("module_3.rs"));
    assert!(rendered.contains("module_5.rs"));
    assert!(rendered.contains("Cross-check the auth entrypoint."));
}

#[test]
fn active_turn_cell_compacts_long_live_planning_sections() {
    let temp = tempdir().unwrap();
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    app.runtime_phase = RuntimePhase::ProcessingResponse;
    app.active_turn = TranscriptTurn {
        entries: vec![TranscriptEntry {
            role: "You".into(),
            message: "Refine the plan".into(),
        }],
    };
    for idx in 1..=5 {
        app.record_planning_action(format!("Inspect planning module {idx}"));
    }
    app.record_planning_note("Reuse the shared auth bridge.");

    let rendered = ActiveTurnCell::new(&app, Some(Path::new(".")))
        .display_lines(100)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains(" Planning "));
    assert!(rendered.contains("... 2 more planning step(s)"));
    assert!(!rendered.contains("planning module 1"));
    assert!(!rendered.contains("planning module 2"));
    assert!(rendered.contains("planning module 3"));
    assert!(rendered.contains("planning module 5"));
    assert!(rendered.contains("Reuse the shared auth bridge."));
}

#[test]
fn active_turn_cell_compacts_long_live_running_sections() {
    let temp = tempdir().unwrap();
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    app.runtime_phase = RuntimePhase::RunningTool;
    app.active_turn = TranscriptTurn {
        entries: vec![TranscriptEntry {
            role: "You".into(),
            message: "Run the checks".into(),
        }],
    };
    for idx in 1..=6 {
        app.record_running_action(format!("Run task {idx}"));
    }

    let rendered = ActiveTurnCell::new(&app, Some(Path::new(".")))
        .display_lines(100)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains(" Running "));
    assert!(rendered.contains("... 2 more running step(s)"));
    assert!(!rendered.contains("Run task 1"));
    assert!(!rendered.contains("Run task 2"));
    assert!(rendered.contains("Run task 3"));
    assert!(rendered.contains("Run task 6"));
}

#[test]
fn active_turn_cell_updated_plan_snapshot() {
    let temp = tempdir().unwrap();
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    app.agent_execution_mode = crate::agent::AgentExecutionMode::Plan;
    app.runtime_phase = RuntimePhase::ProcessingResponse;
    app.runtime_phase_detail = Some("waiting for model response · 3s elapsed".into());
    app.active_turn = TranscriptTurn {
        entries: vec![TranscriptEntry {
            role: "You".into(),
            message: "Read the local codebase and propose the next refactor".into(),
        }],
    };
    app.record_planning_note("The auth flow should reuse codex_login instead of mirroring it.");
    app.record_exploration_action("Read src/oauth.rs");
    app.snapshot = RuntimeSnapshot {
        plan_steps: vec![
            ("completed".into(), "Inspect the current auth bridge".into()),
            ("in_progress".into(), "Replace the bespoke OAuth flow".into()),
            ("pending".into(), "Add snapshot tests for the auth picker".into()),
        ],
        plan_explanation: Some(
            "Prefer direct Codex auth reuse before extending more TUI flows.".into(),
        ),
        ..RuntimeSnapshot::default()
    };

    let rendered = ActiveTurnCell::new(&app, Some(Path::new(".")))
        .display_lines(100)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert_snapshot!("active_turn_cell_updated_plan", rendered);
}

#[test]
fn active_turn_cell_hides_structured_plan_response_once_plan_card_exists() {
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
                message: "Plan the next refactor".into(),
            },
            TranscriptEntry {
                role: "Agent".into(),
                message: "<plan>\n- [completed] Inspect the auth flow\n- [in_progress] Reuse codex_login\n- [pending] Add auth picker snapshots\n</plan>\nPrefer direct auth reuse before expanding more TUI flows.".into(),
            },
        ],
    };
    app.snapshot.plan_steps = vec![
        ("completed".into(), "Inspect the auth flow".into()),
        ("in_progress".into(), "Reuse codex_login".into()),
        ("pending".into(), "Add auth picker snapshots".into()),
    ];
    app.snapshot.plan_explanation =
        Some("Prefer direct auth reuse before expanding more TUI flows.".into());

    let rendered = ActiveTurnCell::new(&app, Some(Path::new(".")))
        .display_lines(100)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("Updated Plan"));
    assert!(!rendered.contains("Responding"));
    assert!(!rendered.contains("<plan>"));
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
fn active_turn_cell_preserves_exploration_agent_exploration_order() {
    let temp = tempdir().unwrap();
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    app.runtime_phase = RuntimePhase::RunningTool;
    app.active_turn = TranscriptTurn {
        entries: vec![
            TranscriptEntry {
                role: "You".into(),
                message: "Inspect the repository".into(),
            },
            TranscriptEntry {
                role: "Tool".into(),
                message: "read_file src/main.rs".into(),
            },
            TranscriptEntry {
                role: "Agent".into(),
                message: "The main entrypoint is thin; I will inspect the runtime bootstrap next."
                    .into(),
            },
            TranscriptEntry {
                role: "Tool".into(),
                message: "read_file src/runtime_context.rs".into(),
            },
        ],
    };

    let rendered = ActiveTurnCell::new(&app, Some(Path::new(".")))
        .display_lines(100)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    let first_exploring = rendered.find(" Exploring ").unwrap();
    let agent = rendered
        .find("• The main entrypoint is thin; I will inspect the runtime bootstrap next.")
        .unwrap();
    let second_exploring = rendered[first_exploring + 1..]
        .find(" Exploring ")
        .map(|idx| first_exploring + 1 + idx)
        .unwrap();

    assert!(rendered.contains("Read src/main.rs"));
    assert!(rendered.contains("Read src/runtime_context.rs"));
    assert!(first_exploring < agent);
    assert!(agent < second_exploring);
}

#[test]
fn active_turn_cell_preserves_agent_then_exploration_order() {
    let temp = tempdir().unwrap();
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    app.runtime_phase = RuntimePhase::RunningTool;
    app.active_turn = TranscriptTurn {
        entries: vec![
            TranscriptEntry {
                role: "You".into(),
                message: "Inspect the bootstrap path".into(),
            },
            TranscriptEntry {
                role: "Agent".into(),
                message: "I have narrowed this down to the runtime bootstrap path.".into(),
            },
            TranscriptEntry {
                role: "Tool".into(),
                message: "read_file src/runtime_context.rs".into(),
            },
        ],
    };

    let rendered = ActiveTurnCell::new(&app, Some(Path::new(".")))
        .display_lines(100)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    let agent = rendered
        .find("• I have narrowed this down to the runtime bootstrap path.")
        .unwrap();
    let exploring = rendered.find(" Exploring ").unwrap();

    assert!(rendered.contains("Read src/runtime_context.rs"));
    assert!(agent < exploring);
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
    assert!(rendered.contains("╭"));
    assert!(rendered.contains("╰"));
    assert!(rendered.contains("• I have inspected"));
    assert!(!rendered.contains("Agent:"));
}

#[test]
fn active_turn_cell_renders_live_response_as_card() {
    let temp = tempdir().unwrap();
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    app.runtime_phase = RuntimePhase::ProcessingResponse;
    app.runtime_phase_detail = Some("waiting for model response".into());
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

    assert!(rendered.contains(" Responding ") || rendered.contains("Responding"));
    assert!(rendered.contains("╭"));
    assert!(rendered.contains("╰"));
    assert!(rendered.contains("• I have inspected the main module"));
}

#[test]
fn active_turn_cell_prefers_responding_over_tool_result_while_processing_response() {
    let temp = tempdir().unwrap();
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    app.runtime_phase = RuntimePhase::ProcessingResponse;
    app.runtime_phase_detail = Some("waiting for model output".into());
    app.active_turn = TranscriptTurn {
        entries: vec![
            TranscriptEntry {
                role: "You".into(),
                message: "Review the repository".into(),
            },
            TranscriptEntry {
                role: "Tool Result".into(),
                message: "bash stdout: partial output".into(),
            },
        ],
    };

    let rendered = ActiveTurnCell::new(&app, Some(Path::new(".")))
        .display_lines(100)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains(" Responding ") || rendered.contains("Responding"));
    assert!(!rendered.contains("Tool Result"));
    assert!(!rendered.contains("bash stdout: partial output"));
}

#[test]
fn active_turn_cell_prefers_responding_over_system_notice_while_sending_prompt() {
    let temp = tempdir().unwrap();
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    app.runtime_phase = RuntimePhase::SendingPrompt;
    app.runtime_phase_detail = Some("sending prompt to provider".into());
    app.active_turn = TranscriptTurn {
        entries: vec![
            TranscriptEntry {
                role: "You".into(),
                message: "Review the repository".into(),
            },
            TranscriptEntry {
                role: "System".into(),
                message: "temporary setup notice".into(),
            },
        ],
    };

    let rendered = ActiveTurnCell::new(&app, Some(Path::new(".")))
        .display_lines(100)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains(" Responding ") || rendered.contains("Responding"));
    assert!(!rendered.contains("temporary setup notice"));
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
fn active_turn_cell_renders_device_code_prompt_system_message() {
    let temp = tempdir().unwrap();
    let mut app = TuiApp::new(ConfigManager {
        path: temp.path().join("config.json"),
    })
    .expect("build tui app");
    app.runtime_phase = RuntimePhase::OAuthPollingDeviceCode;
    app.runtime_phase_detail = Some("Waiting for device-code confirmation.".into());
    app.active_turn = TranscriptTurn {
        entries: vec![
            TranscriptEntry {
                role: "Runtime".into(),
                message: "Starting Codex device-code login flow.".into(),
            },
            TranscriptEntry {
                role: "System".into(),
                message: "Open this URL in a browser and enter the one-time code:\nhttps://example.test\n\nCode: ABCD".into(),
            },
        ],
    };

    let rendered = ActiveTurnCell::new(&app, Some(Path::new(".")))
        .display_lines(100)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("System"));
    assert!(rendered.contains("https://example.test"));
    assert!(rendered.contains("Code: ABCD"));
}
