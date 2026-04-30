use crate::agent::Message;
use crate::context::RetrievalSourceContextEntry;
use crate::prompt::PromptSource;
use crate::workspace::WorkspaceMemory;

pub(crate) fn retrieval_source_entries(
    workspace: &WorkspaceMemory,
    prompt_sources: &[PromptSource],
    history: &[Message],
    session_id: &str,
    vdb_uri: &str,
) -> Vec<RetrievalSourceContextEntry> {
    let workspace_memory_active = prompt_sources
        .iter()
        .any(|source| source.kind_label() == "local_memory");
    let workspace_memory_path = workspace.rara_dir.join("memory.md");
    let workspace_memory_exists = workspace.has_memory_file_cached();
    let workspace_memory_status = if workspace_memory_active {
        "active"
    } else if workspace_memory_exists {
        "available"
    } else {
        "missing"
    };
    let thread_history_status = if history.is_empty() {
        "empty"
    } else {
        "available"
    };
    let vector_memory_status = if vdb_uri.is_empty() {
        "missing"
    } else {
        "available"
    };

    vec![
        RetrievalSourceContextEntry {
            order: 1,
            kind: "workspace_memory".to_string(),
            label: "Workspace Memory".to_string(),
            status: workspace_memory_status.to_string(),
            detail: workspace_memory_path.display().to_string(),
            inclusion_reason: match workspace_memory_status {
                "active" => "included now because the local workspace memory file was discovered as an explicit prompt source".to_string(),
                "available" => "available for future recall or prompt injection, but not active in the current turn".to_string(),
                _ => "no workspace memory file is available for recall or prompt injection".to_string(),
            },
        },
        RetrievalSourceContextEntry {
            order: 2,
            kind: "thread_history".to_string(),
            label: "Thread History".to_string(),
            status: thread_history_status.to_string(),
            detail: format!("session={} messages={}", session_id, history.len()),
            inclusion_reason: if history.is_empty() {
                "no persisted thread history is available for session-local recall yet".to_string()
            } else {
                "available as the session-local history source for restore and future recall surfaces".to_string()
            },
        },
        RetrievalSourceContextEntry {
            order: 3,
            kind: "vector_memory".to_string(),
            label: "Vector Memory Store".to_string(),
            status: vector_memory_status.to_string(),
            detail: vdb_uri.to_string(),
            inclusion_reason: if vector_memory_status == "available" {
                "configured as the durable vector-backed memory store for later retrieval, even though the current recall path is still limited".to_string()
            } else {
                "no vector-backed memory store is configured for retrieval".to_string()
            },
        },
    ]
}
