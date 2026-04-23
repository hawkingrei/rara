use std::sync::Arc;

use serde_json::json;

use super::{
    state_db_status_error, InteractionKind, StateDb, TranscriptTurn, TuiApp,
};
use crate::state_db::{
    PersistedCompactState, PersistedInteraction, PersistedPlanStep, PersistedTurnEntry,
};

impl TuiApp {
    pub fn attach_state_db(&mut self, state_db: Arc<StateDb>) {
        let status = state_db.path().display().to_string();
        self.recent_sessions = state_db.list_recent_sessions(20).unwrap_or_default();
        self.state_db = Some(state_db);
        self.state_db_status = Some(status);
        if !self.snapshot.session_id.is_empty() {
            self.persist_runtime_state();
        }
    }

    pub fn set_state_db_error(&mut self, error: String) {
        self.state_db = None;
        self.state_db_status = Some(state_db_status_error("unavailable", error));
    }

    pub(super) fn persist_runtime_state(&mut self) {
        let Some(state_db) = self.state_db.as_ref() else {
            return;
        };
        if self.snapshot.session_id.is_empty() {
            return;
        }

        if let Err(err) = state_db.upsert_session(
            &self.snapshot.session_id,
            &self.snapshot.cwd,
            &self.snapshot.branch,
            &self.config.provider,
            self.current_model_label(),
            self.config.base_url.as_deref(),
            self.agent_execution_mode_label(),
            self.bash_approval_mode_label(),
            self.snapshot.plan_explanation.as_deref(),
            self.snapshot.history_len,
            self.transcript_entry_count(),
            &PersistedCompactState {
                compaction_count: self.snapshot.compaction_count,
                last_compaction_before_tokens: self.snapshot.last_compaction_before_tokens,
                last_compaction_after_tokens: self.snapshot.last_compaction_after_tokens,
                last_compaction_recent_file_count: Some(
                    self.snapshot.last_compaction_recent_files.len(),
                ),
                last_compaction_boundary_version: self.snapshot.last_compaction_boundary_version,
            },
        ) {
            self.state_db_status = Some(state_db_status_error("write failed", err.to_string()));
            return;
        }

        let plan_steps = self
            .snapshot
            .plan_steps
            .iter()
            .enumerate()
            .map(|(step_index, (status, step))| PersistedPlanStep {
                step_index,
                status: status.clone(),
                step: step.clone(),
            })
            .collect::<Vec<_>>();
        if let Err(err) = state_db.replace_plan_steps(&self.snapshot.session_id, &plan_steps) {
            self.state_db_status =
                Some(state_db_status_error("plan write failed", err.to_string()));
            return;
        }

        let mut interactions = Vec::new();
        for interaction in &self.snapshot.pending_interactions {
            match interaction.kind {
                InteractionKind::RequestInput => {
                    let options_summary = interaction
                        .options
                        .iter()
                        .map(|(label, _)| label.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    let summary = match interaction.note.as_deref() {
                        Some(note) if !note.is_empty() => format!("{options_summary} | {note}"),
                        _ => options_summary,
                    };
                    interactions.push(PersistedInteraction {
                        kind: "request_input".to_string(),
                        status: "pending".to_string(),
                        title: interaction.title.clone(),
                        summary,
                        payload: Some(json!({
                            "question": interaction.title,
                            "options": interaction.options,
                            "note": interaction.note,
                            "source": interaction.source,
                        })),
                    });
                }
                InteractionKind::PlanApproval => {
                    interactions.push(PersistedInteraction {
                        kind: "plan_approval".to_string(),
                        status: "pending".to_string(),
                        title: interaction.title.clone(),
                        summary: interaction.summary.clone(),
                        payload: None,
                    });
                }
                InteractionKind::Approval => {
                    if let Some(approval) = interaction.approval.as_ref() {
                        interactions.push(PersistedInteraction {
                            kind: "approval".to_string(),
                            status: "pending".to_string(),
                            title: interaction.title.clone(),
                            summary: approval.command.clone(),
                            payload: Some(json!({
                                "tool_use_id": approval.tool_use_id,
                                "command": approval.command,
                                "allow_net": approval.allow_net,
                                "program": approval.payload.program,
                                "args": approval.payload.args,
                                "cwd": approval.payload.cwd,
                                "env": approval.payload.env,
                            })),
                        });
                    }
                }
            }
        }
        for interaction in &self.snapshot.completed_interactions {
            let kind = match interaction.kind {
                InteractionKind::RequestInput => "request_input",
                InteractionKind::Approval => "approval",
                InteractionKind::PlanApproval => "plan_approval",
            };
            interactions.push(PersistedInteraction {
                kind: kind.to_string(),
                status: "completed".to_string(),
                title: interaction.title.clone(),
                summary: interaction.summary.clone(),
                payload: None,
            });
        }

        if let Err(err) = state_db.replace_interactions(&self.snapshot.session_id, &interactions) {
            self.state_db_status = Some(state_db_status_error(
                "interaction write failed",
                err.to_string(),
            ));
            return;
        }

        self.recent_sessions = state_db.list_recent_sessions(20).unwrap_or_default();
        self.state_db_status = Some(state_db.path().display().to_string());
    }

    pub(super) fn persist_turn(&mut self, ordinal: usize, turn: &TranscriptTurn) {
        let Some(state_db) = self.state_db.as_ref() else {
            return;
        };
        if self.snapshot.session_id.is_empty() {
            return;
        }
        let entries = turn
            .entries
            .iter()
            .map(|entry| PersistedTurnEntry {
                role: entry.role.clone(),
                message: entry.message.clone(),
            })
            .collect::<Vec<_>>();
        if let Err(err) = state_db.persist_turn(&self.snapshot.session_id, ordinal, &entries) {
            self.state_db_status =
                Some(state_db_status_error("turn write failed", err.to_string()));
        }
    }
}
