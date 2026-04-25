use anyhow::Result;
use uuid::Uuid;

use crate::agent::Message;
use crate::session::{PersistedCompactionEvent, SessionManager};
use crate::state_db::{
    PersistedCompactState, PersistedInteraction, PersistedLegacyRolloutMigration,
    PersistedPlanStep, PersistedPromptRuntimeState, PersistedRecentThreadRecord,
    PersistedRuntimeRolloutItem, PersistedStructuredRolloutEvent, PersistedThreadLineage,
    PersistedThreadRecord, PersistedTurnEntry, PersistedTurnSummary, StateDb,
};

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Default)]
pub struct CompactionRecord {
    pub compaction_count: usize,
    pub before_tokens: Option<usize>,
    pub after_tokens: Option<usize>,
    pub recent_file_count: Option<usize>,
    pub boundary_version: Option<u32>,
    pub recent_files: Vec<String>,
    pub summary: Option<String>,
}

impl From<PersistedCompactState> for CompactionRecord {
    fn from(value: PersistedCompactState) -> Self {
        Self {
            compaction_count: value.compaction_count,
            before_tokens: value.last_compaction_before_tokens,
            after_tokens: value.last_compaction_after_tokens,
            recent_file_count: value.last_compaction_recent_file_count,
            boundary_version: value.last_compaction_boundary_version,
            recent_files: Vec::new(),
            summary: None,
        }
    }
}

impl From<PersistedCompactionEvent> for CompactionRecord {
    fn from(value: PersistedCompactionEvent) -> Self {
        Self {
            compaction_count: value.event_index,
            before_tokens: Some(value.before_tokens),
            after_tokens: Some(value.after_tokens),
            recent_file_count: Some(value.recent_files.len()),
            boundary_version: Some(value.boundary_version),
            recent_files: value.recent_files,
            summary: Some(value.summary),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ThreadMetadata {
    pub session_id: String,
    pub cwd: String,
    pub branch: String,
    pub provider: String,
    pub model: String,
    pub base_url: Option<String>,
    pub agent_mode: String,
    pub bash_approval: String,
    pub created_at: i64,
    pub origin_kind: String,
    pub forked_from_thread_id: Option<String>,
    pub history_len: usize,
    pub transcript_len: usize,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct ThreadSummary {
    pub metadata: ThreadMetadata,
    pub preview: String,
    pub compaction: CompactionRecord,
}

impl From<PersistedThreadRecord> for ThreadMetadata {
    fn from(value: PersistedThreadRecord) -> Self {
        Self {
            session_id: value.session_id,
            cwd: value.cwd,
            branch: value.branch,
            provider: value.provider,
            model: value.model,
            base_url: value.base_url,
            agent_mode: value.agent_mode,
            bash_approval: value.bash_approval,
            created_at: value.created_at,
            origin_kind: value.lineage.origin_kind,
            forked_from_thread_id: value.lineage.forked_from_thread_id,
            history_len: value.history_len,
            transcript_len: value.transcript_len,
            updated_at: value.updated_at,
        }
    }
}

impl From<PersistedRecentThreadRecord> for ThreadSummary {
    fn from(value: PersistedRecentThreadRecord) -> Self {
        Self {
            metadata: ThreadMetadata {
                session_id: value.session_id,
                cwd: value.cwd,
                branch: value.branch,
                provider: value.provider,
                model: value.model,
                base_url: value.base_url,
                agent_mode: value.agent_mode,
                bash_approval: value.bash_approval,
                created_at: value.created_at,
                origin_kind: value.lineage.origin_kind,
                forked_from_thread_id: value.lineage.forked_from_thread_id,
                history_len: value.history_len,
                transcript_len: value.transcript_len,
                updated_at: value.updated_at,
            },
            preview: value.preview,
            compaction: CompactionRecord {
                compaction_count: value.compaction_count,
                before_tokens: value.last_compaction_before_tokens,
                after_tokens: value.last_compaction_after_tokens,
                recent_file_count: value.last_compaction_recent_file_count,
                boundary_version: value.last_compaction_boundary_version,
                recent_files: Vec::new(),
                summary: None,
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct RolloutTurnItem {
    pub summary: PersistedTurnSummary,
    pub entries: Vec<PersistedTurnEntry>,
}

#[derive(Debug, Clone)]
pub enum RolloutItem {
    Compaction(CompactionRecord),
    PlanState {
        explanation: Option<String>,
        steps: Vec<PersistedPlanStep>,
    },
    Interaction(PersistedInteraction),
    Turn(RolloutTurnItem),
}

#[derive(Debug, Clone)]
pub struct ThreadSnapshot {
    pub metadata: ThreadMetadata,
    pub history: Vec<Message>,
    pub compaction: CompactionRecord,
    pub plan_explanation: Option<String>,
    pub plan_steps: Vec<PersistedPlanStep>,
    pub interactions: Vec<PersistedInteraction>,
    pub rollout_items: Vec<RolloutItem>,
}

#[derive(Debug, Clone)]
struct ThreadMaterializedState {
    metadata: ThreadMetadata,
    history: Vec<Message>,
    compaction: CompactionRecord,
    plan_explanation: Option<String>,
    plan_steps: Vec<PersistedPlanStep>,
    interactions: Vec<PersistedInteraction>,
    rollout_items: Vec<RolloutItem>,
}

#[derive(Debug, Clone, Default)]
struct LegacyNonTurnRolloutMigration {
    structured_events: Vec<PersistedStructuredRolloutEvent>,
    runtime_rollout: Vec<PersistedRuntimeRolloutItem>,
    compaction_events: Vec<PersistedCompactionEvent>,
}

pub struct ThreadStore<'a> {
    session_manager: &'a SessionManager,
    state_db: &'a StateDb,
}

impl<'a> ThreadStore<'a> {
    pub fn list_recent_threads_for_db(
        state_db: &StateDb,
        limit: usize,
    ) -> Result<Vec<ThreadSummary>> {
        state_db
            .list_recent_thread_records(limit)
            .map(|threads| threads.into_iter().map(ThreadSummary::from).collect())
    }

    pub fn new(session_manager: &'a SessionManager, state_db: &'a StateDb) -> Self {
        Self {
            session_manager,
            state_db,
        }
    }

    pub fn latest_thread_id(&self) -> Result<Option<String>> {
        self.state_db.latest_thread_id()
    }

    pub fn list_recent_threads(&self, limit: usize) -> Result<Vec<ThreadSummary>> {
        Self::list_recent_threads_for_db(self.state_db, limit)
    }

    pub fn load_thread(&self, session_id: &str) -> Result<ThreadSnapshot> {
        let materialized = self.materialize_thread_state(session_id)?;
        Ok(ThreadSnapshot {
            metadata: materialized.metadata,
            history: materialized.history,
            compaction: materialized.compaction,
            plan_explanation: materialized.plan_explanation,
            plan_steps: materialized.plan_steps,
            interactions: materialized.interactions,
            rollout_items: materialized.rollout_items,
        })
    }

    pub fn fork_thread(&self, source_thread_id: &str) -> Result<String> {
        let materialized = self.materialize_thread_state(source_thread_id)?;
        let runtime_state = self
            .state_db
            .load_session_runtime_state(source_thread_id)?
            .unwrap_or_default();
        let compact_state = compact_state_from_record(&materialized.compaction);
        let forked_thread_id = Uuid::new_v4().to_string();
        let lineage = PersistedThreadLineage {
            origin_kind: "fork".to_string(),
            forked_from_thread_id: Some(source_thread_id.to_string()),
        };

        self.session_manager
            .save_session(&forked_thread_id, &materialized.history)?;
        self.state_db.upsert_session_with_lineage(
            &forked_thread_id,
            &materialized.metadata.cwd,
            &materialized.metadata.branch,
            &materialized.metadata.provider,
            &materialized.metadata.model,
            materialized.metadata.base_url.as_deref(),
            &materialized.metadata.agent_mode,
            &materialized.metadata.bash_approval,
            &lineage,
            materialized.plan_explanation.as_deref(),
            &runtime_state.prompt_runtime,
            materialized.history.len(),
            materialized.metadata.transcript_len,
            &compact_state,
        )?;

        let recorder = ThreadRecorder::new(self.state_db);
        recorder.replace_plan_steps(&forked_thread_id, &materialized.plan_steps)?;
        recorder.replace_interactions(&forked_thread_id, &materialized.interactions)?;

        for compaction in self
            .session_manager
            .load_compaction_events(source_thread_id)?
        {
            self.state_db.append_compaction_rollout_event(
                &forked_thread_id,
                compaction.event_index,
                compaction.before_tokens,
                compaction.after_tokens,
                compaction.boundary_version,
                &compaction.recent_files,
                &compaction.summary,
            )?;
        }

        recorder.replace_runtime_rollout_events(
            &forked_thread_id,
            &[PersistedStructuredRolloutEvent::RuntimeState {
                explanation: materialized.plan_explanation.clone(),
                steps: materialized.plan_steps.clone(),
                interactions: materialized.interactions.clone(),
            }],
        )?;

        for summary in self.state_db.load_turn_summaries(source_thread_id)? {
            let entries = self
                .state_db
                .load_turn_entries(source_thread_id, summary.ordinal)?;
            recorder.persist_turn(&forked_thread_id, summary.ordinal, &entries)?;
        }

        Ok(forked_thread_id)
    }

    fn materialize_thread_state(&self, session_id: &str) -> Result<ThreadMaterializedState> {
        let history = match self
            .session_manager
            .load_thread_history_migration(session_id)
        {
            Ok(migration) => migration.history,
            Err(err) if SessionManager::is_missing_thread_history_error(&err) => Vec::new(),
            Err(err) => return Err(err),
        };
        let metadata = self
            .state_db
            .load_thread_record(session_id)?
            .ok_or_else(|| anyhow::anyhow!("Thread {session_id} not found in state db"))?;
        let LegacyNonTurnRolloutMigration {
            structured_events,
            runtime_rollout: migration_runtime_rollout,
            compaction_events,
        } = self.load_legacy_non_turn_rollout_migration(session_id)?;
        let compaction = compaction_events
            .last()
            .cloned()
            .map(CompactionRecord::from)
            .unwrap_or_else(|| {
                self.state_db
                    .load_session_compact_state(session_id)
                    .map(CompactionRecord::from)
                    .unwrap_or_default()
            });
        let mut plan_explanation = self.state_db.load_session_plan_explanation(session_id)?;
        let mut plan_steps = self.state_db.load_plan_steps(session_id)?;
        let mut interactions = self.state_db.load_interactions(session_id)?;
        let turn_items = self
            .state_db
            .load_turn_summaries(session_id)?
            .into_iter()
            .map(|summary| {
                let entries = self
                    .state_db
                    .load_turn_entries(session_id, summary.ordinal)?;
                Ok(RolloutItem::Turn(RolloutTurnItem { summary, entries }))
            })
            .collect::<Result<Vec<_>>>()?;
        let mut rollout_items = Vec::new();
        if structured_events.is_empty() && compaction.compaction_count > 0 {
            rollout_items.push(RolloutItem::Compaction(compaction.clone()));
        }
        let mut saw_runtime_state = false;
        let mut saw_plan_state = false;
        let mut saw_interaction = false;
        let mut structured_plan_explanation = None;
        let mut structured_plan_steps = Vec::new();
        let mut structured_interactions = Vec::new();
        for item in structured_events {
            match item {
                PersistedStructuredRolloutEvent::Compaction {
                    event_index,
                    before_tokens,
                    after_tokens,
                    boundary_version,
                    recent_files,
                    summary,
                } => rollout_items.push(RolloutItem::Compaction(CompactionRecord {
                    compaction_count: event_index,
                    before_tokens: Some(before_tokens),
                    after_tokens: Some(after_tokens),
                    recent_file_count: Some(recent_files.len()),
                    boundary_version: Some(boundary_version),
                    recent_files,
                    summary: Some(summary),
                })),
                PersistedStructuredRolloutEvent::RuntimeState {
                    explanation,
                    steps,
                    interactions: runtime_interactions,
                } => {
                    saw_runtime_state = true;
                    saw_plan_state = true;
                    saw_interaction = true;
                    plan_explanation = explanation.clone();
                    plan_steps = steps.clone();
                    interactions = runtime_interactions.clone();
                    if !steps.is_empty() || explanation.is_some() {
                        rollout_items.push(RolloutItem::PlanState { explanation, steps });
                    }
                    rollout_items.extend(
                        runtime_interactions
                            .into_iter()
                            .map(RolloutItem::Interaction),
                    );
                }
                PersistedStructuredRolloutEvent::PlanState { explanation, steps } => {
                    saw_plan_state = true;
                    structured_plan_explanation = explanation.clone();
                    structured_plan_steps = steps.clone();
                    rollout_items.push(RolloutItem::PlanState { explanation, steps });
                }
                PersistedStructuredRolloutEvent::Interaction(interaction) => {
                    saw_interaction = true;
                    structured_interactions.push(interaction.clone());
                    rollout_items.push(RolloutItem::Interaction(interaction));
                }
            }
        }

        if !saw_runtime_state {
            if saw_plan_state {
                plan_explanation = structured_plan_explanation;
                plan_steps = structured_plan_steps;
            }
            if saw_interaction {
                interactions = structured_interactions;
            }
        }

        let legacy_runtime_rollout = if saw_runtime_state || (saw_plan_state && saw_interaction) {
            Vec::new()
        } else {
            migration_runtime_rollout
        };
        let legacy_plan_state = legacy_runtime_rollout.iter().find_map(|item| match item {
            PersistedRuntimeRolloutItem::PlanState { explanation, steps } => {
                Some((explanation.clone(), steps.clone()))
            }
            PersistedRuntimeRolloutItem::Interaction(_) => None,
        });
        let legacy_interactions = legacy_runtime_rollout
            .iter()
            .filter_map(|item| match item {
                PersistedRuntimeRolloutItem::Interaction(interaction) => Some(interaction.clone()),
                PersistedRuntimeRolloutItem::PlanState { .. } => None,
            })
            .collect::<Vec<_>>();

        if saw_runtime_state {
            // Append-only runtime snapshots already defined the current plan/interaction state.
        } else if !saw_plan_state && !saw_interaction && legacy_runtime_rollout.is_empty() {
            if !plan_steps.is_empty() || plan_explanation.is_some() {
                rollout_items.push(RolloutItem::PlanState {
                    explanation: plan_explanation.clone(),
                    steps: plan_steps.clone(),
                });
            }
            rollout_items.extend(interactions.iter().cloned().map(RolloutItem::Interaction));
        } else if !saw_plan_state && !saw_interaction {
            if let Some((explanation, steps)) = legacy_plan_state.clone() {
                plan_explanation = explanation;
                plan_steps = steps;
            }
            interactions = legacy_interactions.clone();
            rollout_items.extend(
                legacy_runtime_rollout
                    .iter()
                    .cloned()
                    .map(|item| match item {
                        PersistedRuntimeRolloutItem::PlanState { explanation, steps } => {
                            RolloutItem::PlanState { explanation, steps }
                        }
                        PersistedRuntimeRolloutItem::Interaction(interaction) => {
                            RolloutItem::Interaction(interaction)
                        }
                    }),
            );
        } else {
            if !saw_plan_state {
                if legacy_runtime_rollout.is_empty() {
                    if !plan_steps.is_empty() || plan_explanation.is_some() {
                        rollout_items.push(RolloutItem::PlanState {
                            explanation: plan_explanation.clone(),
                            steps: plan_steps.clone(),
                        });
                    }
                } else {
                    if let Some((explanation, steps)) = legacy_plan_state.clone() {
                        plan_explanation = explanation;
                        plan_steps = steps;
                    }
                    rollout_items.extend(legacy_runtime_rollout.iter().filter_map(
                        |item| match item {
                            PersistedRuntimeRolloutItem::PlanState { explanation, steps } => {
                                Some(RolloutItem::PlanState {
                                    explanation: explanation.clone(),
                                    steps: steps.clone(),
                                })
                            }
                            PersistedRuntimeRolloutItem::Interaction(_) => None,
                        },
                    ));
                }
            }
            if !saw_interaction {
                if legacy_runtime_rollout.is_empty() {
                    rollout_items
                        .extend(interactions.iter().cloned().map(RolloutItem::Interaction));
                } else {
                    interactions = legacy_interactions.clone();
                    rollout_items.extend(legacy_runtime_rollout.into_iter().filter_map(|item| {
                        match item {
                            PersistedRuntimeRolloutItem::Interaction(interaction) => {
                                Some(RolloutItem::Interaction(interaction))
                            }
                            PersistedRuntimeRolloutItem::PlanState { .. } => None,
                        }
                    }));
                }
            }
        }
        rollout_items.extend(turn_items);

        Ok(ThreadMaterializedState {
            metadata: metadata.into(),
            history,
            compaction,
            plan_explanation,
            plan_steps,
            interactions,
            rollout_items,
        })
    }

    fn load_legacy_non_turn_rollout_migration(
        &self,
        session_id: &str,
    ) -> Result<LegacyNonTurnRolloutMigration> {
        let PersistedLegacyRolloutMigration {
            structured_events,
            runtime_rollout,
        } = self.state_db.load_legacy_rollout_migration(session_id)?;
        let compaction_events = self.session_manager.load_compaction_events(session_id)?;
        Ok(LegacyNonTurnRolloutMigration {
            structured_events,
            runtime_rollout,
            compaction_events,
        })
    }
}

pub struct ThreadRuntimeState<'a> {
    pub session_id: &'a str,
    pub cwd: &'a str,
    pub branch: &'a str,
    pub provider: &'a str,
    pub model: &'a str,
    pub base_url: Option<&'a str>,
    pub agent_mode: &'a str,
    pub bash_approval: &'a str,
    pub plan_explanation: Option<&'a str>,
    pub prompt_runtime: PersistedPromptRuntimeState,
    pub history_len: usize,
    pub transcript_len: usize,
    pub compact_state: PersistedCompactState,
}

pub struct ThreadRecorder<'a> {
    state_db: &'a StateDb,
}

impl<'a> ThreadRecorder<'a> {
    pub fn new(state_db: &'a StateDb) -> Self {
        Self { state_db }
    }

    pub fn persist_runtime_state(&self, state: &ThreadRuntimeState<'_>) -> Result<()> {
        self.state_db.upsert_session(
            state.session_id,
            state.cwd,
            state.branch,
            state.provider,
            state.model,
            state.base_url,
            state.agent_mode,
            state.bash_approval,
            state.plan_explanation,
            &state.prompt_runtime,
            state.history_len,
            state.transcript_len,
            &state.compact_state,
        )
    }

    pub fn replace_plan_steps(&self, session_id: &str, steps: &[PersistedPlanStep]) -> Result<()> {
        self.state_db.replace_plan_steps(session_id, steps)
    }

    pub fn replace_interactions(
        &self,
        session_id: &str,
        interactions: &[PersistedInteraction],
    ) -> Result<()> {
        self.state_db.replace_interactions(session_id, interactions)
    }

    pub fn replace_runtime_rollout_events(
        &self,
        session_id: &str,
        items: &[PersistedStructuredRolloutEvent],
    ) -> Result<()> {
        self.state_db
            .replace_runtime_rollout_events(session_id, items)
    }

    pub fn persist_turn(
        &self,
        session_id: &str,
        ordinal: usize,
        entries: &[PersistedTurnEntry],
    ) -> Result<PersistedTurnSummary> {
        self.state_db.persist_turn(session_id, ordinal, entries)
    }
}

fn compact_state_from_record(record: &CompactionRecord) -> PersistedCompactState {
    PersistedCompactState {
        compaction_count: record.compaction_count,
        last_compaction_before_tokens: record.before_tokens,
        last_compaction_after_tokens: record.after_tokens,
        last_compaction_recent_file_count: record
            .recent_file_count
            .or(Some(record.recent_files.len()).filter(|value| *value > 0)),
        last_compaction_boundary_version: record.boundary_version,
    }
}
