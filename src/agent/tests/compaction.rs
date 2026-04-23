use std::sync::Arc;

use serde_json::json;

use crate::agent::{Agent, Message};
use crate::session::SessionManager;
use crate::tool::ToolManager;
use crate::vectordb::VectorDB;
use crate::workspace::WorkspaceMemory;

use super::support::SequencedBackend;

#[tokio::test]
async fn manual_compact_replaces_older_history_with_summary() {
    let backend = Arc::new(SequencedBackend::new(Vec::new()));
    let mut agent = Agent::new(
        ToolManager::new(),
        backend,
        Arc::new(VectorDB::new("data/lancedb")),
        Arc::new(SessionManager::new().expect("session manager")),
        Arc::new(WorkspaceMemory::new().expect("workspace memory")),
    );
    agent.history = vec![
        Message {
            role: "user".to_string(),
            content: json!("inspect the repo"),
        },
        Message {
            role: "assistant".to_string(),
            content: json!("I checked Cargo.toml and src/main.rs"),
        },
    ];

    let compacted = agent
        .compact_now_with_reporter(|_| {})
        .await
        .expect("compact should succeed");

    assert!(compacted);
    assert_eq!(agent.compact_state.compaction_count, 1);
    assert_eq!(agent.history[0].role, "system");
    let boundary = agent.history[0]
        .content
        .as_object()
        .expect("compact boundary");
    assert_eq!(
        boundary.get("type").and_then(serde_json::Value::as_str),
        Some("compact_boundary")
    );
    assert!(agent.history[1]
        .content
        .to_string()
        .contains("STRUCTURED SUMMARY OF PREVIOUS CONVERSATION"));
}

#[tokio::test]
async fn manual_compact_carries_recent_files_forward() {
    let backend = Arc::new(SequencedBackend::new(Vec::new()));
    let mut agent = Agent::new(
        ToolManager::new(),
        backend,
        Arc::new(VectorDB::new("data/lancedb")),
        Arc::new(SessionManager::new().expect("session manager")),
        Arc::new(WorkspaceMemory::new().expect("workspace memory")),
    );
    agent.history = vec![
        Message {
            role: "assistant".to_string(),
            content: json!([
                {"type":"tool_use","id":"tool-1","name":"read_file","input":{"path":"src/main.rs"}},
                {"type":"tool_use","id":"tool-2","name":"list_files","input":{"path":"src/agent"}}
            ]),
        },
        Message {
            role: "user".to_string(),
            content: json!([
                {"type":"tool_result","tool_use_id":"tool-1","content":"fn main() {}"},
                {"type":"tool_result","tool_use_id":"tool-2","content":"planning.rs"}
            ]),
        },
        Message {
            role: "assistant".to_string(),
            content: json!("I inspected the relevant files."),
        },
    ];

    let compacted = agent
        .compact_now_with_reporter(|_| {})
        .await
        .expect("compact should succeed");

    assert!(compacted);
    assert_eq!(agent.history[2].role, "system");
    assert_eq!(agent.history[3].role, "system");
    let boundary = agent.history[0]
        .content
        .as_object()
        .expect("compact boundary");
    assert_eq!(
        boundary
            .get("recent_file_count")
            .and_then(serde_json::Value::as_u64),
        Some(2)
    );
    let recent_files = agent.history[2].content.to_string();
    assert!(recent_files.contains("RECENT FILES FROM COMPACTED HISTORY"));
    assert!(recent_files.contains("src/main.rs"));
    assert!(recent_files.contains("src/agent"));
    let excerpts = agent.history[3].content.to_string();
    assert!(excerpts.contains("RECENT FILE EXCERPTS FROM COMPACTED HISTORY"));
    assert!(excerpts.contains("### src/main.rs"));
    assert!(excerpts.contains("fn main() {}"));
}

#[tokio::test]
async fn manual_compact_prefers_latest_excerpt_and_tracks_apply_patch() {
    let backend = Arc::new(SequencedBackend::new(Vec::new()));
    let mut agent = Agent::new(
        ToolManager::new(),
        backend,
        Arc::new(VectorDB::new("data/lancedb")),
        Arc::new(SessionManager::new().expect("session manager")),
        Arc::new(WorkspaceMemory::new().expect("workspace memory")),
    );
    agent.history = vec![
        Message {
            role: "assistant".to_string(),
            content: json!([
                {"type":"tool_use","id":"tool-1","name":"read_file","input":{"path":"src/main.rs","start_line":1,"end_line":2}},
                {"type":"tool_use","id":"tool-2","name":"apply_patch","input":{"path":"src/lib.rs"}}
            ]),
        },
        Message {
            role: "user".to_string(),
            content: json!([
                {"type":"tool_result","tool_use_id":"tool-1","content":"old snippet"},
                {"type":"tool_result","tool_use_id":"tool-2","content":"patch applied"}
            ]),
        },
        Message {
            role: "assistant".to_string(),
            content: json!([
                {"type":"tool_use","id":"tool-3","name":"read_file","input":{"path":"src/main.rs","start_line":10,"end_line":12}}
            ]),
        },
        Message {
            role: "user".to_string(),
            content: json!([
                {"type":"tool_result","tool_use_id":"tool-3","content":"new snippet"}
            ]),
        },
        Message {
            role: "assistant".to_string(),
            content: json!("I updated the inspection notes."),
        },
    ];

    let compacted = agent
        .compact_now_with_reporter(|_| {})
        .await
        .expect("compact should succeed");

    assert!(compacted);
    let recent_files = agent.history[2].content.to_string();
    assert!(recent_files.contains("src/lib.rs"));
    let excerpts = agent.history[3].content.to_string();
    assert!(excerpts.contains("new snippet"));
    assert!(!excerpts.contains("old snippet"));
    assert!(excerpts.contains("lines 10-12"));
}
