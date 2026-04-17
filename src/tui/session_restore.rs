use std::sync::Arc;

use anyhow::Result;

use crate::agent::{
    Agent, CompletedInteraction, PendingApproval, PendingUserInput, PlanStep, PlanStepStatus,
};
use crate::state_db::StateDb;

use super::state::{TranscriptEntry, TranscriptTurn, TuiApp};

pub(super) fn restore_latest_session(
    state_db: &Arc<StateDb>,
    app: &mut TuiApp,
    agent_slot: &mut Option<Agent>,
) -> Result<()> {
    let Some(session_id) = state_db.latest_session_id()? else {
        return Ok(());
    };
    restore_session_by_id(session_id.as_str(), app, agent_slot)
}

pub(super) fn restore_session_by_id(
    session_id: &str,
    app: &mut TuiApp,
    agent_slot: &mut Option<Agent>,
) -> Result<()> {
    let Some(agent) = agent_slot.as_mut() else {
        return Ok(());
    };
    let Some(state_db) = app.state_db.as_ref() else {
        return Ok(());
    };
    if let Ok(history) = agent.session_manager.load_session(session_id) {
        agent.history = history;
        agent.session_id = session_id.to_string();
    }
    let persisted_steps = state_db.load_plan_steps(session_id)?;
    if !persisted_steps.is_empty() {
        agent.current_plan = persisted_steps
            .into_iter()
            .map(|step| PlanStep {
                step: step.step,
                status: match step.status.as_str() {
                    "completed" => PlanStepStatus::Completed,
                    "in_progress" => PlanStepStatus::InProgress,
                    _ => PlanStepStatus::Pending,
                },
            })
            .collect();
    } else {
        agent.current_plan.clear();
    }
    agent.plan_explanation = state_db.load_session_plan_explanation(session_id)?;
    agent.pending_user_input = None;
    agent.pending_approval = None;
    agent.completed_user_input = None;
    agent.completed_approval = None;
    let interactions = state_db.load_interactions(session_id)?;
    for interaction in interactions {
        match (interaction.kind.as_str(), interaction.status.as_str()) {
            ("request_input", "pending") => {
                let Some(payload) = interaction.payload.as_ref() else {
                    continue;
                };
                let options = payload
                    .get("options")
                    .and_then(|value| value.as_array())
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| {
                                let pair = item.as_array()?;
                                let label = pair.first()?.as_str()?.to_string();
                                let detail = pair.get(1)?.as_str()?.to_string();
                                Some((label, detail))
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                agent.pending_user_input = Some(PendingUserInput {
                    question: payload
                        .get("question")
                        .and_then(|value| value.as_str())
                        .unwrap_or(&interaction.title)
                        .to_string(),
                    options,
                    note: payload
                        .get("note")
                        .and_then(|value| value.as_str())
                        .map(str::to_string),
                });
            }
            ("approval", "pending") => {
                let payload = interaction.payload.as_ref();
                let command = payload
                    .and_then(|payload| payload.get("command"))
                    .and_then(|value| value.as_str())
                    .unwrap_or(&interaction.summary)
                    .to_string();
                agent.pending_approval = Some(PendingApproval {
                    tool_use_id: payload
                        .and_then(|payload| payload.get("tool_use_id"))
                        .and_then(|value| value.as_str())
                        .unwrap_or("restored")
                        .to_string(),
                    command,
                    allow_net: payload
                        .and_then(|payload| payload.get("allow_net"))
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false),
                });
            }
            ("request_input", "completed") => {
                agent.completed_user_input = Some(CompletedInteraction {
                    title: interaction.title,
                    summary: interaction.summary,
                });
            }
            ("approval", "completed") => {
                agent.completed_approval = Some(CompletedInteraction {
                    title: interaction.title,
                    summary: interaction.summary,
                });
            }
            _ => {}
        }
    }
    let summaries = state_db.load_turn_summaries(session_id)?;
    let mut turns = Vec::with_capacity(summaries.len());
    for summary in summaries {
        let entries = state_db
            .load_turn_entries(session_id, summary.ordinal)?
            .into_iter()
            .map(|entry| TranscriptEntry {
                role: entry.role,
                message: entry.message,
            })
            .collect::<Vec<_>>();
        if !entries.is_empty() {
            turns.push(TranscriptTurn { entries });
        }
    }
    if !turns.is_empty() {
        app.restore_committed_turns(turns);
    } else {
        app.reset_transcript();
    }
    app.sync_snapshot(agent);
    app.notice = Some(format!("Resumed session {session_id}."));
    Ok(())
}

pub(crate) fn provider_requires_api_key(provider: &str) -> bool {
    !matches!(
        provider,
        "mock" | "local" | "local-candle" | "gemma4" | "qwen3" | "qwn3" | "ollama"
    )
}
