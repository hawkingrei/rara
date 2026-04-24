use std::sync::Arc;

use anyhow::Result;

use crate::agent::{
    latest_compact_boundary_metadata, Agent, CompactBoundaryMetadata, CompletedInteraction,
    PendingApproval, PendingUserInput, PlanStep, PlanStepStatus,
};
use crate::state_db::StateDb;
use crate::thread_store::{CompactionRecord, RolloutItem, ThreadStore};
use crate::tools::bash::BashCommandInput;

use super::state::{TranscriptEntry, TranscriptTurn, TuiApp};

pub(super) fn restore_latest_thread(
    state_db: &Arc<StateDb>,
    app: &mut TuiApp,
    agent_slot: &mut Option<Agent>,
) -> Result<()> {
    let Some(agent) = agent_slot.as_ref() else {
        return Ok(());
    };
    let store = ThreadStore::new(agent.session_manager.as_ref(), state_db.as_ref());
    let Some(thread_id) = store.latest_thread_id()? else {
        return Ok(());
    };
    restore_thread_by_id(thread_id.as_str(), app, agent_slot)
}

pub(super) fn restore_thread_by_id(
    thread_id: &str,
    app: &mut TuiApp,
    agent_slot: &mut Option<Agent>,
) -> Result<()> {
    let Some(agent) = agent_slot.as_mut() else {
        return Ok(());
    };
    let Some(state_db) = app.state_db.as_ref() else {
        return Ok(());
    };
    let thread_store = ThreadStore::new(agent.session_manager.as_ref(), state_db.as_ref());
    let thread = thread_store.load_thread(thread_id)?;
    let crate::thread_store::ThreadSnapshot {
        metadata,
        history,
        compaction,
        plan_explanation,
        plan_steps,
        interactions,
        rollout_items,
    } = thread;
    agent.history = history;
    agent.session_id = metadata.session_id;
    apply_compaction_record(agent, &compaction);
    agent.compact_state.last_compaction_boundary = match compaction.boundary_version {
            Some(version) => Some(CompactBoundaryMetadata {
                version,
                before_tokens: compaction.before_tokens.unwrap_or_default(),
                recent_file_count: compaction.recent_file_count.unwrap_or_default(),
            }),
            None => latest_compact_boundary_metadata(&agent.history),
        };
    if !plan_steps.is_empty() {
        agent.current_plan = plan_steps
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
    agent.plan_explanation = plan_explanation;
    agent.pending_user_input = None;
    agent.pending_approval = None;
    agent.completed_user_input = None;
    agent.completed_approval = None;
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
                let request = payload
                    .cloned()
                    .map(BashCommandInput::from_value)
                    .transpose()
                    .unwrap_or(None)
                    .unwrap_or(BashCommandInput {
                        command: Some(command.clone()),
                        program: None,
                        args: Vec::new(),
                        cwd: None,
                        env: Default::default(),
                        allow_net: payload
                            .and_then(|payload| payload.get("allow_net"))
                            .and_then(|value| value.as_bool())
                            .unwrap_or(false),
                    });
                agent.pending_approval = Some(PendingApproval {
                    tool_use_id: payload
                        .and_then(|payload| payload.get("tool_use_id"))
                        .and_then(|value| value.as_str())
                        .unwrap_or("restored")
                        .to_string(),
                    request,
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
    let mut turns = Vec::new();
    for item in rollout_items {
        match item {
            RolloutItem::Turn(turn) if !turn.entries.is_empty() => {
                let entries = turn
                    .entries
                    .into_iter()
                    .map(|entry| TranscriptEntry {
                        role: entry.role,
                        message: entry.message,
                    })
                    .collect::<Vec<_>>();
                turns.push(TranscriptTurn { entries });
            }
            RolloutItem::Turn(_)
            | RolloutItem::Compaction(_)
            | RolloutItem::PlanState { .. }
            | RolloutItem::Interaction(_) => {}
        }
    }
    if !turns.is_empty() {
        app.restore_committed_turns(turns);
    } else {
        app.reset_transcript();
    }
    app.sync_snapshot(agent);
    app.notice = Some(format!("Resumed thread {thread_id}."));
    Ok(())
}

fn apply_compaction_record(agent: &mut Agent, compaction: &CompactionRecord) {
    agent.compact_state.compaction_count = compaction.compaction_count;
    agent.compact_state.last_compaction_before_tokens = compaction.before_tokens;
    agent.compact_state.last_compaction_after_tokens = compaction.after_tokens;
}

pub(crate) fn provider_requires_api_key(provider: &str) -> bool {
    !matches!(
        provider,
        "mock" | "local" | "local-candle" | "gemma4" | "qwen3" | "qwn3" | "ollama"
    )
}
