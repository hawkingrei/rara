use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::json;

use crate::agent::{Agent, AgentEvent, CompactState, ContentBlock, Message};
use crate::llm::{ContextBudget, LlmBackend, LlmResponse, TokenUsage};
use crate::session::SessionManager;
use crate::tool::ToolManager;
use crate::vectordb::VectorDB;
use crate::workspace::WorkspaceMemory;

use super::support::SequencedBackend;

struct SlowSummarizeBackend;

struct TinyBudgetSummaryBackend;

#[async_trait]
impl LlmBackend for SlowSummarizeBackend {
    async fn ask(
        &self,
        _messages: &[Message],
        _tools: &[serde_json::Value],
    ) -> Result<LlmResponse> {
        Ok(LlmResponse {
            content: vec![ContentBlock::Text {
                text: "query completed".to_string(),
            }],
            stop_reason: Some("end_turn".to_string()),
            usage: Some(TokenUsage::default()),
        })
    }

    async fn embed(&self, _text: &str) -> Result<Vec<f32>> {
        Ok(vec![0.0; 8])
    }

    async fn summarize(&self, _messages: &[Message], _instruction: &str) -> Result<String> {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        Ok("slow summary".to_string())
    }
}

#[async_trait]
impl LlmBackend for TinyBudgetSummaryBackend {
    async fn ask(
        &self,
        _messages: &[Message],
        _tools: &[serde_json::Value],
    ) -> Result<LlmResponse> {
        Ok(LlmResponse {
            content: vec![ContentBlock::Text {
                text: "query completed".to_string(),
            }],
            stop_reason: Some("end_turn".to_string()),
            usage: Some(TokenUsage::default()),
        })
    }

    async fn embed(&self, _text: &str) -> Result<Vec<f32>> {
        Ok(vec![0.0; 8])
    }

    async fn summarize(&self, _messages: &[Message], _instruction: &str) -> Result<String> {
        Ok("summary".to_string())
    }

    fn context_budget(
        &self,
        _messages: &[Message],
        _tools: &[serde_json::Value],
    ) -> Option<ContextBudget> {
        Some(ContextBudget {
            context_window_tokens: 16,
            reserved_output_tokens: 4,
            compact_threshold_tokens: 1,
        })
    }
}

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
    assert!(
        agent.history[1]
            .content
            .to_string()
            .contains("STRUCTURED SUMMARY OF PREVIOUS CONVERSATION")
    );
}

#[tokio::test]
async fn automatic_compaction_timeout_does_not_block_query() {
    let backend = Arc::new(SlowSummarizeBackend);
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
            content: json!("x".repeat(50_000)),
        },
        Message {
            role: "assistant".to_string(),
            content: json!("y".repeat(50_000)),
        },
    ];

    let mut statuses = Vec::new();
    agent
        .query_with_mode_and_events(
            "continue".to_string(),
            crate::agent::AgentOutputMode::Silent,
            |event| {
                if let AgentEvent::Status(status) = event {
                    statuses.push(status);
                }
            },
        )
        .await
        .expect("query should continue after automatic compaction timeout");

    assert!(
        statuses
            .iter()
            .any(|status| status.contains("Automatic history compaction timed out"))
    );
    assert!(
        agent
            .history
            .last()
            .is_some_and(|message| message.content.to_string().contains("query completed"))
    );
}

#[tokio::test]
async fn automatic_compaction_failure_suspends_retry_until_history_grows() {
    let backend = Arc::new(SlowSummarizeBackend);
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
            content: json!("x".repeat(50_000)),
        },
        Message {
            role: "assistant".to_string(),
            content: json!("y".repeat(50_000)),
        },
    ];

    agent
        .compact_if_needed_with_reporter(|_| {})
        .await
        .expect("automatic compaction timeout should be non-fatal");
    let after_failure = agent.compact_state.clone();
    assert_eq!(after_failure.consecutive_auto_compaction_failures, 1);
    assert!(after_failure.auto_compaction_retry_after_tokens.is_some());

    let mut statuses = Vec::new();
    agent
        .compact_if_needed_with_reporter(|event| {
            if let AgentEvent::Status(status) = event {
                statuses.push(status);
            }
        })
        .await
        .expect("suspended auto compaction should be non-fatal");

    assert!(
        statuses
            .iter()
            .any(|status| status.contains("temporarily suspended"))
    );
    assert_eq!(
        agent.compact_state.consecutive_auto_compaction_failures,
        after_failure.consecutive_auto_compaction_failures
    );
    assert_eq!(
        agent.compact_state.auto_compaction_retry_after_tokens,
        after_failure.auto_compaction_retry_after_tokens
    );
}

#[tokio::test]
async fn successful_compaction_clears_auto_failure_backoff() {
    let backend = Arc::new(SequencedBackend::new(Vec::new()));
    let mut agent = Agent::new(
        ToolManager::new(),
        backend,
        Arc::new(VectorDB::new("data/lancedb")),
        Arc::new(SessionManager::new().expect("session manager")),
        Arc::new(WorkspaceMemory::new().expect("workspace memory")),
    );
    agent.compact_state = CompactState {
        consecutive_auto_compaction_failures: 2,
        auto_compaction_retry_after_tokens: Some(100_000),
        ..Default::default()
    };
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
        .expect("manual compaction should succeed");

    assert!(compacted);
    assert_eq!(agent.compact_state.consecutive_auto_compaction_failures, 0);
    assert_eq!(agent.compact_state.auto_compaction_retry_after_tokens, None);
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
                {"type":"tool_use","id":"tool-3","name":"read_file","input":{"path":"src/main.rs","offset":10,"limit":3}}
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

#[tokio::test]
async fn manual_compact_preserves_recent_api_round_pair() {
    let backend = Arc::new(TinyBudgetSummaryBackend);
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
            content: json!("inspect old state"),
        },
        Message {
            role: "assistant".to_string(),
            content: json!([
                {"type":"tool_use","id":"tool-old","name":"read_file","input":{"path":"src/old.rs"}}
            ]),
        },
        Message {
            role: "user".to_string(),
            content: json!([
                {"type":"tool_result","tool_use_id":"tool-old","content":"old output"}
            ]),
        },
        Message {
            role: "assistant".to_string(),
            content: json!([
                {"type":"tool_use","id":"tool-recent","name":"read_file","input":{"path":"src/recent.rs"}}
            ]),
        },
        Message {
            role: "user".to_string(),
            content: json!([
                {"type":"tool_result","tool_use_id":"tool-recent","content":"recent output"}
            ]),
        },
    ];

    let compacted = agent
        .compact_now_with_reporter(|_| {})
        .await
        .expect("compact should succeed");

    assert!(compacted);
    let recent_tool_use_index = agent
        .history
        .iter()
        .position(|message| message.content.to_string().contains("tool-recent"))
        .expect("recent tool use should be retained");
    assert_eq!(agent.history[recent_tool_use_index].role, "assistant");
    assert_eq!(
        agent.history[recent_tool_use_index + 1].role,
        "user",
        "tool result should stay with the retained assistant API round"
    );
    assert!(
        agent.history[recent_tool_use_index + 1]
            .content
            .to_string()
            .contains("tool-recent")
    );
}
