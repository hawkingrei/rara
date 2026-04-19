use secrecy::{ExposeSecret, SecretString};
use std::sync::Arc;
use std::time::Instant;

use anyhow::anyhow;
use tokio::sync::mpsc;

use crate::agent::{Agent, AgentOutputMode, BashApprovalMode};
use crate::llm::LlmBackend;
use crate::oauth::OAuthManager;
use crate::redaction::sanitize_url_for_display;
use crate::sandbox::SandboxManager;
use crate::session::SessionManager;
use crate::skill::SkillManager;
use crate::tool::ToolManager;
use crate::tools::agent::{AgentTool, ExploreAgentTool, PlanAgentTool, TeamCreateTool};
use crate::tools::bash::BashTool;
use crate::tools::context::RetrieveSessionContextTool;
use crate::tools::file::{ListFilesTool, ReadFileTool, ReplaceTool, WriteFileTool};
use crate::tools::patch::ApplyPatchTool;
use crate::tools::search::{GlobTool, GrepTool};
use crate::tools::skill::SkillTool;
use crate::tools::vector::{RememberExperienceTool, RetrieveExperienceTool};
use crate::tools::web::WebFetchTool;
use crate::tools::workspace::UpdateProjectMemoryTool;
use crate::vectordb::VectorDB;
use crate::workspace::WorkspaceMemory;

use super::super::state::{
    OAuthLoginMode, RunningTask, RuntimePhase, TaskCompletion, TaskKind, TuiApp, TuiEvent,
};
use super::events::{apply_tui_event, convert_agent_event, format_error_chain};

fn restore_execute_mode_after_plan_turn(app: &mut TuiApp, agent: &mut Agent) {
    if matches!(
        app.agent_execution_mode,
        crate::agent::AgentExecutionMode::Plan
    ) {
        app.set_agent_execution_mode(crate::agent::AgentExecutionMode::Execute);
        agent.set_execution_mode(crate::agent::AgentExecutionMode::Execute);
    }
}

fn try_start_queued_follow_up(app: &mut TuiApp, agent_slot: &mut Option<Agent>) {
    if app.running_task.is_some()
        || app.active_pending_interaction().is_some()
        || app.has_pending_planning_suggestion()
    {
        return;
    }

    let Some(prompt) = app.pop_queued_follow_up_message() else {
        return;
    };
    let Some(agent) = agent_slot.take() else {
        app.queue_follow_up_message(prompt);
        return;
    };

    let remaining = app.queued_follow_up_count();
    app.notice = Some(if remaining > 0 {
        format!("Running queued follow-up. {remaining} more queued.")
    } else {
        "Running queued follow-up.".to_string()
    });
    start_query_task(app, prompt, agent);
}

pub(super) fn start_query_task(app: &mut TuiApp, prompt: String, mut agent: Agent) {
    let (sender, receiver) = mpsc::unbounded_channel();
    app.clear_pending_planning_suggestion();
    app.clear_active_live_sections();
    agent.set_execution_mode(app.agent_execution_mode);
    agent.set_bash_approval_mode(app.bash_approval_mode);
    app.notice = Some("Running prompt.".into());
    app.set_runtime_phase(RuntimePhase::SendingPrompt, Some("sending prompt".into()));
    app.push_entry("You", prompt.clone());

    let handle = tokio::spawn(async move {
        let tx = sender.clone();
        let result = agent
            .query_with_mode_and_events(prompt, AgentOutputMode::Silent, move |event| {
                if let Some(event) = convert_agent_event(event) {
                    let _ = tx.send(event);
                }
            })
            .await;
        TaskCompletion::Query { agent, result }
    });

    app.running_task = Some(RunningTask {
        kind: TaskKind::Query,
        receiver,
        handle,
        started_at: Instant::now(),
        next_heartbeat_after_secs: 2,
    });
}

pub(super) fn should_suggest_planning_mode(app: &TuiApp, prompt: &str) -> bool {
    if app.is_busy()
        || app.has_pending_plan_approval()
        || app.has_pending_approval()
        || app.pending_request_input().is_some()
        || matches!(
            app.agent_execution_mode,
            crate::agent::AgentExecutionMode::Plan
        )
    {
        return false;
    }

    let trimmed = prompt.trim();
    if trimmed.is_empty() || trimmed.lines().count() > 20 {
        return false;
    }

    let lowered = trimmed.to_ascii_lowercase();
    let strong_keywords = [
        "review the code",
        "review this repo",
        "inspect the codebase",
        "implementation plan",
        "refactor",
        "architecture",
        "design proposal",
        "migration plan",
        "跨模块",
        "重构",
        "架构",
        "设计方案",
        "实现方案",
        "修改建议",
        "看一下代码",
        "代码质量",
    ];
    if strong_keywords
        .iter()
        .any(|keyword| lowered.contains(keyword) || trimmed.contains(keyword))
    {
        return true;
    }

    let asks_for_analysis = lowered.contains("analyze")
        || lowered.contains("analyse")
        || lowered.contains("proposal")
        || lowered.contains("suggest")
        || lowered.contains("review")
        || lowered.contains("plan")
        || lowered.contains("design");
    let mentions_codebase = lowered.contains("repo")
        || lowered.contains("repository")
        || lowered.contains("codebase")
        || lowered.contains("module")
        || lowered.contains("system")
        || trimmed.contains("代码")
        || trimmed.contains("仓库")
        || trimmed.contains("项目");

    asks_for_analysis && mentions_codebase
}

pub(super) fn start_compact_task(app: &mut TuiApp, mut agent: Agent) {
    let (sender, receiver) = mpsc::unbounded_channel();
    agent.set_execution_mode(app.agent_execution_mode);
    agent.set_bash_approval_mode(app.bash_approval_mode);
    app.notice = Some("Compacting conversation history.".into());
    app.set_runtime_phase(
        RuntimePhase::ProcessingResponse,
        Some("compacting history".into()),
    );
    app.push_entry("You", "/compact");

    let handle = tokio::spawn(async move {
        let tx = sender.clone();
        let result = agent
            .compact_now_with_reporter(move |event| {
                if let Some(event) = convert_agent_event(event) {
                    let _ = tx.send(event);
                }
            })
            .await;
        TaskCompletion::Compact { agent, result }
    });

    app.running_task = Some(RunningTask {
        kind: TaskKind::Compact,
        receiver,
        handle,
        started_at: Instant::now(),
        next_heartbeat_after_secs: 2,
    });
}

pub(super) fn start_pending_approval_task(
    app: &mut TuiApp,
    selection: BashApprovalMode,
    mut agent: Agent,
) {
    let (sender, receiver) = mpsc::unbounded_channel();
    let selection_label = match selection {
        BashApprovalMode::Once => "run once",
        BashApprovalMode::Always => "always allow bash",
        BashApprovalMode::Suggestion => "suggestion only",
    };
    app.notice = Some(format!("Answering approval request: {selection_label}."));
    app.set_runtime_phase(
        RuntimePhase::ProcessingResponse,
        Some("resuming after approval".into()),
    );

    let handle = tokio::spawn(async move {
        let tx = sender.clone();
        let result = agent
            .answer_pending_approval_with_events(selection, AgentOutputMode::Silent, move |event| {
                if let Some(event) = convert_agent_event(event) {
                    let _ = tx.send(event);
                }
            })
            .await;
        TaskCompletion::Query { agent, result }
    });

    app.running_task = Some(RunningTask {
        kind: TaskKind::Query,
        receiver,
        handle,
        started_at: Instant::now(),
        next_heartbeat_after_secs: 2,
    });
}

pub(super) fn start_plan_approval_resume_task(
    app: &mut TuiApp,
    continue_planning: bool,
    mut agent: Agent,
) {
    let (sender, receiver) = mpsc::unbounded_channel();
    app.clear_active_live_sections();
    app.set_pending_plan_approval(false);
    app.record_completed_interaction(
        crate::tui::state::InteractionKind::PlanApproval,
        "Plan Decision",
        if continue_planning {
            "Continued planning"
        } else {
            "Approved and started implementation"
        },
        None,
    );
    app.notice = Some(if continue_planning {
        "Continuing plan refinement.".into()
    } else {
        "Plan approved. Continuing with implementation.".into()
    });
    app.set_runtime_phase(
        RuntimePhase::ProcessingResponse,
        Some(if continue_planning {
            "resuming plan refinement".into()
        } else {
            "resuming approved plan".into()
        }),
    );

    agent.set_execution_mode(if continue_planning {
        crate::agent::AgentExecutionMode::Plan
    } else {
        crate::agent::AgentExecutionMode::Execute
    });
    app.set_agent_execution_mode(agent.execution_mode);

    let handle = tokio::spawn(async move {
        let tx = sender.clone();
        let result = agent
            .resume_after_plan_approval_with_events(
                continue_planning,
                AgentOutputMode::Silent,
                move |event| {
                    if let Some(event) = convert_agent_event(event) {
                        let _ = tx.send(event);
                    }
                },
            )
            .await;
        TaskCompletion::Query { agent, result }
    });

    app.running_task = Some(RunningTask {
        kind: TaskKind::Query,
        receiver,
        handle,
        started_at: Instant::now(),
        next_heartbeat_after_secs: 2,
    });
}

pub(super) fn start_rebuild_task(app: &mut TuiApp) {
    let (sender, receiver) = mpsc::unbounded_channel();
    let config = app.config.clone();
    let provider = config.provider.clone();
    let model = config.model.clone().unwrap_or_else(|| "-".to_string());
    app.notice = Some(format!("Rebuilding backend for {provider} / {model}."));
    app.set_runtime_phase(
        RuntimePhase::RebuildingBackend,
        Some(format!("preparing {provider} / {model}")),
    );
    app.push_entry("Download", format!("Preparing {} / {}", provider, model));

    let handle = tokio::spawn(async move {
        let tx = sender.clone();
        let progress: crate::local_backend::LocalProgressReporter = Arc::new(move |message| {
            let _ = tx.send(TuiEvent::Transcript {
                role: "Download",
                message,
            });
        });
        let result = rebuild_agent_with_progress(&config, Some(progress)).await;
        TaskCompletion::Rebuild { result }
    });

    app.running_task = Some(RunningTask {
        kind: TaskKind::Rebuild,
        receiver,
        handle,
        started_at: Instant::now(),
        next_heartbeat_after_secs: u64::MAX,
    });
}

pub(super) fn start_oauth_task(
    app: &mut TuiApp,
    oauth_manager: Arc<OAuthManager>,
    mode: OAuthLoginMode,
) {
    let (sender, receiver) = mpsc::unbounded_channel();
    let mode_label = match mode {
        OAuthLoginMode::Browser => "browser login",
        OAuthLoginMode::DeviceCode => "device-code login",
    };
    app.notice = Some(format!("Starting Codex {mode_label}."));
    app.set_runtime_phase(
        RuntimePhase::OAuthStarting,
        Some(format!("starting {mode_label}")),
    );
    app.push_entry("Runtime", format!("Starting Codex {mode_label} flow."));

    let handle = tokio::spawn(async move {
        let result = run_oauth_login(oauth_manager, mode, sender.clone()).await;
        TaskCompletion::OAuth { mode, result }
    });

    app.running_task = Some(RunningTask {
        kind: TaskKind::OAuth,
        receiver,
        handle,
        started_at: Instant::now(),
        next_heartbeat_after_secs: u64::MAX,
    });
}

pub(super) async fn finish_running_task_if_ready(
    app: &mut TuiApp,
    agent_slot: &mut Option<Agent>,
) -> anyhow::Result<()> {
    if app.running_task.is_none() {
        return Ok(());
    }

    let (pending_events, is_finished) = {
        let task = app.running_task.as_mut().expect("task should exist");
        let mut pending_events = Vec::new();
        while let Ok(event) = task.receiver.try_recv() {
            pending_events.push(event);
        }
        let is_finished = task.handle.is_finished();
        (pending_events, is_finished)
    };

    for event in pending_events {
        apply_tui_event(app, event);
    }

    if !is_finished {
        emit_query_heartbeat(app);
        return Ok(());
    }

    let mut task = app.running_task.take().expect("task should exist");
    let completion = task.handle.await?;
    while let Ok(event) = task.receiver.try_recv() {
        apply_tui_event(app, event);
    }
    match completion {
        TaskCompletion::Query { agent, result } => {
            let mut agent = agent;
            match result {
                Ok(_) => {
                    let finished_plan_turn = matches!(
                        app.agent_execution_mode,
                        crate::agent::AgentExecutionMode::Plan
                    );
                    app.clear_active_live_sections();
                    if finished_plan_turn {
                        restore_execute_mode_after_plan_turn(app, &mut agent);
                        app.set_pending_plan_approval(
                            agent.last_query_produced_plan() && !agent.current_plan.is_empty(),
                        );
                    }
                    *agent_slot = Some(agent);
                    if let Some(agent) = agent_slot.as_ref() {
                        app.sync_snapshot(agent);
                    }
                    app.finalize_agent_stream(None);
                    if finished_plan_turn && app.has_pending_plan_approval() {
                        app.notice = Some("Plan ready for approval.".into());
                        app.set_runtime_phase(
                            RuntimePhase::Idle,
                            Some("awaiting plan approval".into()),
                        );
                    } else {
                        if finished_plan_turn {
                            app.push_notice("Planning finished. Returned to execute mode.");
                        }
                        app.finalize_active_turn();
                        app.notice = Some("Prompt finished.".into());
                        app.set_runtime_phase(RuntimePhase::Idle, Some("prompt finished".into()));
                        try_start_queued_follow_up(app, agent_slot);
                    }
                }
                Err(err) => {
                    restore_execute_mode_after_plan_turn(app, &mut agent);
                    app.clear_active_live_sections();
                    app.set_pending_plan_approval(false);
                    *agent_slot = Some(agent);
                    if let Some(agent) = agent_slot.as_ref() {
                        app.sync_snapshot(agent);
                    }
                    app.finalize_agent_stream(None);
                    app.set_runtime_phase(RuntimePhase::Failed, Some("query failed".into()));
                    let mut message = format!("Query failed:\n{}", format_error_chain(&err));
                    if app.config.provider == "ollama" {
                        let base_url = app
                            .config
                            .base_url
                            .as_deref()
                            .unwrap_or("http://localhost:11434");
                        message.push_str(&format!(
                            "\nbase_url={}",
                            sanitize_url_for_display(base_url)
                        ));
                    }
                    app.push_entry("System", message.clone());
                    app.push_notice(message);
                }
            }
        }
        TaskCompletion::Compact { agent, result } => {
            *agent_slot = Some(agent);
            if let Some(agent) = agent_slot.as_ref() {
                app.sync_snapshot(agent);
            }
            match result {
                Ok(true) => {
                    app.clear_active_live_sections();
                    if let Some((before, after)) = app
                        .snapshot
                        .last_compaction_before_tokens
                        .zip(app.snapshot.last_compaction_after_tokens)
                    {
                        let message = format!(
                            "Conversation compacted.\nEstimated history tokens: {before} -> {after}"
                        );
                        app.push_entry("Agent", message.clone());
                        app.push_notice(message);
                    } else {
                        app.push_entry("Agent", "Conversation compacted.");
                        app.push_notice("Conversation compacted.");
                    }
                    app.finalize_active_turn();
                    app.set_runtime_phase(RuntimePhase::Idle, Some("history compacted".into()));
                    try_start_queued_follow_up(app, agent_slot);
                }
                Ok(false) => {
                    app.clear_active_live_sections();
                    let message = "Conversation history did not need compaction.";
                    app.push_entry("Agent", message);
                    app.push_notice(message);
                    app.finalize_active_turn();
                    app.set_runtime_phase(RuntimePhase::Idle, Some("compact skipped".into()));
                    try_start_queued_follow_up(app, agent_slot);
                }
                Err(err) => {
                    app.clear_active_live_sections();
                    app.set_runtime_phase(RuntimePhase::Failed, Some("compact failed".into()));
                    let message = format!("Compaction failed:\n{}", format_error_chain(&err));
                    app.push_entry("System", message.clone());
                    app.push_notice(message);
                }
            }
        }
        TaskCompletion::Rebuild { result } => match result {
            Ok(agent) => {
                let mut agent = agent;
                agent.set_execution_mode(app.agent_execution_mode);
                agent.set_bash_approval_mode(app.bash_approval_mode);
                app.config_manager.save(&app.config)?;
                app.setup_status = Some(format!(
                    "Applied {} / {}",
                    app.config.provider,
                    app.current_model_label()
                ));
                app.notice = app.setup_status.clone();
                let queued_follow_up_messages = std::mem::take(&mut app.queued_follow_up_messages);
                app.reset_transcript();
                app.queued_follow_up_messages = queued_follow_up_messages;
                *agent_slot = Some(agent);
                if let Some(agent) = agent_slot.as_ref() {
                    app.sync_snapshot(agent);
                }
                app.close_overlay();
                app.set_runtime_phase(RuntimePhase::BackendReady, Some("backend ready".into()));
                app.push_entry("Runtime", app.setup_status.clone().unwrap_or_default());
                app.finalize_active_turn();
                try_start_queued_follow_up(app, agent_slot);
            }
            Err(err) => {
                app.set_runtime_phase(RuntimePhase::Failed, Some("backend rebuild failed".into()));
                let message = format!("Failed to apply config:\n{}", format_error_chain(&err));
                app.setup_status = Some(message.clone());
                app.push_notice(message);
            }
        },
        TaskCompletion::OAuth { mode, result } => match result {
            Ok(access_token) => {
                app.config
                    .set_api_key(access_token.expose_secret().to_string());
                app.config.provider = "codex".into();
                if app.config.model.is_none() {
                    app.config.model = Some("codex".into());
                }
                app.config_manager.save(&app.config)?;
                let saved_message = match mode {
                    OAuthLoginMode::Browser => "Saved browser login token to local config.",
                    OAuthLoginMode::DeviceCode => "Saved device-code login token to local config.",
                };
                app.setup_status = Some(saved_message.into());
                app.notice = app.setup_status.clone();
                app.set_runtime_phase(RuntimePhase::OAuthSaved, Some("oauth token saved".into()));
                app.overlay = None;
                app.push_entry("Runtime", saved_message);
                start_rebuild_task(app);
            }
            Err(err) => {
                app.set_runtime_phase(RuntimePhase::Failed, Some("oauth failed".into()));
                let message = format!("OAuth failed:\n{}", format_error_chain(&err));
                app.push_entry("System", message.clone());
                app.push_notice(message);
            }
        },
    }

    Ok(())
}

fn emit_query_heartbeat(app: &mut TuiApp) {
    let Some(task) = app.running_task.as_mut() else {
        return;
    };
    if !matches!(task.kind, TaskKind::Query) {
        return;
    }

    let elapsed = task.started_at.elapsed().as_secs();
    if elapsed < task.next_heartbeat_after_secs {
        return;
    }

    let is_local = super::super::command::is_local_provider(&app.config.provider);
    let detail = if is_local {
        format!("local model is still generating · {}s elapsed", elapsed)
    } else {
        format!("waiting for model response · {}s elapsed", elapsed)
    };
    task.next_heartbeat_after_secs = elapsed.saturating_add(1);

    app.set_runtime_phase(RuntimePhase::SendingPrompt, Some(detail.clone()));
    app.notice = Some(if is_local {
        format!("Working locally · {}s elapsed", elapsed)
    } else {
        format!("Waiting on {} · {}s elapsed", app.config.provider, elapsed)
    });
}

async fn run_oauth_login(
    oauth_manager: Arc<OAuthManager>,
    mode: OAuthLoginMode,
    sender: mpsc::UnboundedSender<TuiEvent>,
) -> anyhow::Result<SecretString> {
    match mode {
        OAuthLoginMode::Browser => {
            let (verifier, challenge) = oauth_manager.generate_pkce();
            let (port, receiver) = oauth_manager.start_callback_server().await?;
            let auth_url = oauth_manager.get_authorize_url(&challenge, port);
            let is_ssh = super::super::is_ssh_session();
            let _ = sender.send(TuiEvent::Transcript {
                role: "Runtime",
                message: if is_ssh {
                    format!(
                        "SSH session detected. Browser login is not reliable from a remote shell because the callback listens on localhost:{port}.\nUse device-code login or API key instead, or open this URL from the same machine running the TUI:\n{auth_url}"
                    )
                } else {
                    format!(
                        "Starting Codex browser login.\nOpen this URL if the browser does not launch automatically:\n{auth_url}"
                    )
                },
            });
            if is_ssh {
                return Err(anyhow!(
                    "browser login is unavailable in SSH/headless sessions; use device-code login or API key instead"
                ));
            }
            let _ = open::that(&auth_url);

            let _ = sender.send(TuiEvent::Transcript {
                role: "Runtime",
                message: "Waiting for browser callback.".into(),
            });
            let code = receiver.await?;
            let _ = sender.send(TuiEvent::Transcript {
                role: "Runtime",
                message: "Received browser callback, exchanging token.".into(),
            });
            let token = oauth_manager.exchange_code(&code, &verifier, port).await?;
            Ok(token.access_token)
        }
        OAuthLoginMode::DeviceCode => {
            let device_code = oauth_manager.request_device_code().await?;
            let _ = sender.send(TuiEvent::Transcript {
                role: "Runtime",
                message: format!(
                    "Starting Codex device-code login.\nOpen this URL in a browser and enter the one-time code:\n{}\n\nCode: {}",
                    device_code.verification_url, device_code.user_code
                ),
            });
            let token = oauth_manager
                .complete_device_code_login(&device_code)
                .await?;
            Ok(token.access_token)
        }
    }
}

async fn rebuild_agent_with_progress(
    config: &crate::config::RaraConfig,
    progress: Option<crate::local_backend::LocalProgressReporter>,
) -> anyhow::Result<Agent> {
    let backend = crate::build_backend_with_progress(config, progress).await?;
    let backend_arc: Arc<dyn LlmBackend> = backend.into();

    let vdb = Arc::new(VectorDB::new("data/lancedb"));
    let session_manager = Arc::new(SessionManager::new()?);
    let workspace = Arc::new(WorkspaceMemory::new()?);
    let sandbox_manager = Arc::new(SandboxManager::new()?);

    let mut skill_manager = SkillManager::new();
    let _ = skill_manager.load_all();
    let skill_manager_arc = Arc::new(skill_manager);

    let tool_manager = create_full_tool_manager(
        backend_arc.clone(),
        vdb.clone(),
        session_manager.clone(),
        workspace.clone(),
        sandbox_manager.clone(),
        skill_manager_arc,
        crate::prompt::PromptRuntimeConfig::from_config(config),
    );

    let mut agent = Agent::new(tool_manager, backend_arc, vdb, session_manager, workspace);
    agent.set_prompt_config(crate::prompt::PromptRuntimeConfig::from_config(config));
    Ok(agent)
}

fn create_full_tool_manager(
    backend: Arc<dyn LlmBackend>,
    vdb: Arc<VectorDB>,
    session_manager: Arc<SessionManager>,
    workspace: Arc<WorkspaceMemory>,
    sandbox: Arc<SandboxManager>,
    skill_manager: Arc<SkillManager>,
    prompt_config: crate::prompt::PromptRuntimeConfig,
) -> ToolManager {
    let mut tm = ToolManager::new();
    tm.register(Box::new(BashTool {
        sandbox: sandbox.clone(),
    }));
    tm.register(Box::new(ReadFileTool));
    tm.register(Box::new(ApplyPatchTool));
    tm.register(Box::new(WriteFileTool));
    tm.register(Box::new(ListFilesTool));
    tm.register(Box::new(ReplaceTool));
    tm.register(Box::new(WebFetchTool));
    tm.register(Box::new(GlobTool));
    tm.register(Box::new(GrepTool));
    tm.register(Box::new(RememberExperienceTool {
        backend: backend.clone(),
        db_uri: "data/lancedb".into(),
    }));
    tm.register(Box::new(RetrieveExperienceTool {
        backend: backend.clone(),
        db_uri: "data/lancedb".into(),
    }));
    tm.register(Box::new(RetrieveSessionContextTool {
        backend: backend.clone(),
        vdb: vdb.clone(),
        session_manager: session_manager.clone(),
    }));
    tm.register(Box::new(UpdateProjectMemoryTool {
        workspace: workspace.clone(),
    }));
    tm.register(Box::new(SkillTool {
        skill_manager: skill_manager.clone(),
    }));
    tm.register(Box::new(AgentTool {
        backend: backend.clone(),
        vdb: vdb.clone(),
        session_manager: session_manager.clone(),
        workspace: workspace.clone(),
        prompt_config: prompt_config.clone(),
    }));
    tm.register(Box::new(ExploreAgentTool {
        backend: backend.clone(),
        vdb: vdb.clone(),
        session_manager: session_manager.clone(),
        workspace: workspace.clone(),
        prompt_config: prompt_config.clone(),
    }));
    tm.register(Box::new(PlanAgentTool {
        backend: backend.clone(),
        vdb: vdb.clone(),
        session_manager: session_manager.clone(),
        workspace: workspace.clone(),
        prompt_config,
    }));
    tm.register(Box::new(TeamCreateTool {
        backend,
        vdb,
        session_manager,
        workspace,
    }));
    tm
}

#[cfg(test)]
mod tests {
    use crate::config::ConfigManager;
    use crate::tui::state::TuiApp;
    use tempfile::tempdir;

    use super::should_suggest_planning_mode;

    #[test]
    fn suggests_planning_for_repo_review_requests() {
        let temp = tempdir().unwrap();
        let app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("build tui app");
        assert!(should_suggest_planning_mode(
            &app,
            "看一下代码，并提出修改建议"
        ));
        assert!(should_suggest_planning_mode(
            &app,
            "Review this repository and propose architectural improvements."
        ));
    }

    #[test]
    fn skips_planning_for_simple_requests() {
        let temp = tempdir().unwrap();
        let app = TuiApp::new(ConfigManager {
            path: temp.path().join("config.json"),
        })
        .expect("build tui app");
        assert!(!should_suggest_planning_mode(
            &app,
            "Fix the typo in README."
        ));
        assert!(!should_suggest_planning_mode(
            &app,
            "What does this function do?"
        ));
    }
}
