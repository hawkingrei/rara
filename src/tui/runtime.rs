use std::sync::Arc;

use tokio::sync::mpsc;

use crate::agent::{Agent, AgentEvent, AgentOutputMode};
use crate::llm::LlmBackend;
use crate::oauth::OAuthManager;
use crate::sandbox::SandboxManager;
use crate::session::SessionManager;
use crate::skill::SkillManager;
use crate::tool::ToolManager;
use crate::tools::agent::{AgentTool, TeamCreateTool};
use crate::tools::bash::BashTool;
use crate::tools::context::RetrieveSessionContextTool;
use crate::tools::file::{
    ListFilesTool, ReadFileTool, ReplaceTool, SearchFilesTool, WriteFileTool,
};
use crate::tools::search::{GlobTool, GrepTool};
use crate::tools::skill::SkillTool;
use crate::tools::vector::{RememberExperienceTool, RetrieveExperienceTool};
use crate::tools::web::WebFetchTool;
use crate::tools::workspace::UpdateProjectMemoryTool;
use crate::vectordb::VectorDB;
use crate::workspace::WorkspaceMemory;

use super::command::{model_help_text, resolve_model_selection};
use super::state::{
    HelpTab, LocalCommand, LocalCommandKind, Overlay, RunningTask, TaskCompletion, TaskKind,
    TuiApp, TuiEvent,
};

pub async fn execute_local_command(
    command: LocalCommand,
    app: &mut TuiApp,
    agent_slot: &mut Option<Agent>,
    oauth_manager: &Arc<OAuthManager>,
) -> anyhow::Result<()> {
    match command.kind {
        LocalCommandKind::Help => app.open_overlay(Overlay::Help(HelpTab::General)),
        LocalCommandKind::Status => app.open_overlay(Overlay::Status),
        LocalCommandKind::Clear => app.reset_transcript(),
        LocalCommandKind::Setup => app.open_overlay(Overlay::Setup),
        LocalCommandKind::Model => handle_model_command(command.arg.as_deref(), app)?,
        LocalCommandKind::Login => {
            if app.is_busy() {
                app.push_notice("A task is already running. Wait for it to finish.");
            } else {
                start_oauth_task(app, Arc::clone(oauth_manager));
            }
        }
    }
    if let Some(agent) = agent_slot.as_ref() {
        app.sync_snapshot(agent);
    }
    Ok(())
}

pub fn start_query_task(app: &mut TuiApp, prompt: String, mut agent: Agent) {
    let (sender, receiver) = mpsc::unbounded_channel();
    app.notice = Some("Running prompt.".into());
    app.push_entry("You", prompt.clone());

    let handle = tokio::spawn(async move {
        let tx = sender.clone();
        let result = agent
            .query_with_mode_and_events(prompt, AgentOutputMode::Silent, move |event| {
                let _ = tx.send(convert_agent_event(event));
            })
            .await;
        TaskCompletion::Query { agent, result }
    });

    app.running_task = Some(RunningTask {
        kind: TaskKind::Query,
        receiver,
        handle,
    });
}

pub fn start_rebuild_task(app: &mut TuiApp) {
    let (sender, receiver) = mpsc::unbounded_channel();
    let config = app.config.clone();
    let provider = config.provider.clone();
    let model = config.model.clone().unwrap_or_else(|| "-".to_string());
    app.notice = Some(format!("Rebuilding backend for {provider} / {model}."));
    app.push_entry("Runtime", format!("Reloading backend for {provider} / {model}."));

    let handle = tokio::spawn(async move {
        let tx = sender.clone();
        let progress: crate::local_backend::LocalProgressReporter = Arc::new(move |message| {
            let _ = tx.send(TuiEvent::Transcript {
                role: "Runtime",
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
    });
}

pub fn start_oauth_task(app: &mut TuiApp, oauth_manager: Arc<OAuthManager>) {
    let (sender, receiver) = mpsc::unbounded_channel();
    app.notice = Some("Starting OAuth login.".into());
    app.push_entry("Runtime", "Starting OAuth login flow.");

    let handle = tokio::spawn(async move {
        let result = run_oauth_login(oauth_manager, sender.clone()).await;
        TaskCompletion::OAuth { result }
    });

    app.running_task = Some(RunningTask {
        kind: TaskKind::OAuth,
        receiver,
        handle,
    });
}

pub async fn finish_running_task_if_ready(
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
        return Ok(());
    }

    let task = app.running_task.take().expect("task should exist");
    let completion = task.handle.await?;
    match completion {
        TaskCompletion::Query { agent, result } => {
            *agent_slot = Some(agent);
            if let Some(agent) = agent_slot.as_ref() {
                app.sync_snapshot(agent);
            }
            match result {
                Ok(_) => {
                    app.notice = Some("Prompt finished.".into());
                }
                Err(err) => {
                    app.push_notice(format!("Query failed: {err}"));
                }
            }
        }
        TaskCompletion::Rebuild { result } => match result {
            Ok(agent) => {
                app.config_manager.save(&app.config)?;
                app.setup_status = Some(format!(
                    "Applied {} / {}",
                    app.config.provider,
                    app.current_model_label()
                ));
                app.notice = app.setup_status.clone();
                app.transcript.clear();
                *agent_slot = Some(agent);
                if let Some(agent) = agent_slot.as_ref() {
                    app.sync_snapshot(agent);
                }
                app.close_overlay();
                app.push_entry("Runtime", app.setup_status.clone().unwrap_or_default());
            }
            Err(err) => {
                let message = format!("Failed to apply config: {err}");
                app.setup_status = Some(message.clone());
                app.push_notice(message);
            }
        },
        TaskCompletion::OAuth { result } => match result {
            Ok(access_token) => {
                app.config.api_key = Some(access_token);
                app.config.provider = "codex_oauth".into();
                app.config_manager.save(&app.config)?;
                app.setup_status = Some("Saved OAuth token.".into());
                app.notice = app.setup_status.clone();
                app.close_overlay();
                app.push_entry("Runtime", "Saved OAuth token.");
            }
            Err(err) => {
                app.push_notice(format!("OAuth failed: {err}"));
            }
        },
    }

    Ok(())
}

fn handle_model_command(arg: Option<&str>, app: &mut TuiApp) -> anyhow::Result<()> {
    let Some(raw_arg) = arg.map(str::trim).filter(|arg| !arg.is_empty()) else {
        app.open_overlay(Overlay::ModelPicker);
        app.notice = Some("Opened model picker.".into());
        return Ok(());
    };

    if raw_arg == "list" {
        app.push_notice(model_help_text(app));
        return Ok(());
    }

    let Some(idx) = resolve_model_selection(raw_arg, app) else {
        app.push_notice(format!("Unknown model preset '{raw_arg}'. Try /model or /help."));
        return Ok(());
    };

    app.select_local_model(idx);
    start_rebuild_task(app);
    Ok(())
}

fn apply_tui_event(app: &mut TuiApp, event: TuiEvent) {
    match event {
        TuiEvent::Transcript { role, message } => app.push_entry(role, message),
    }
}

fn convert_agent_event(event: AgentEvent) -> TuiEvent {
    match event {
        AgentEvent::Status(message) => TuiEvent::Transcript {
            role: "Status",
            message,
        },
        AgentEvent::AssistantText(text) => TuiEvent::Transcript {
            role: "Agent",
            message: text,
        },
        AgentEvent::ToolUse { name, input } => TuiEvent::Transcript {
            role: "Tool",
            message: format!("{name} {input}"),
        },
        AgentEvent::ToolResult {
            name,
            content,
            is_error,
        } => TuiEvent::Transcript {
            role: if is_error { "Tool Error" } else { "Tool Result" },
            message: format!("{name}: {content}"),
        },
    }
}

async fn run_oauth_login(
    oauth_manager: Arc<OAuthManager>,
    sender: mpsc::UnboundedSender<TuiEvent>,
) -> anyhow::Result<String> {
    let (verifier, challenge) = oauth_manager.generate_pkce();
    let (port, receiver) = oauth_manager.start_callback_server().await?;
    let _ = sender.send(TuiEvent::Transcript {
        role: "Runtime",
        message: format!("Opened OAuth flow on localhost:{port}."),
    });
    let _ = open::that(oauth_manager.get_authorize_url(&challenge, port));

    let code = receiver.await?;
    let _ = sender.send(TuiEvent::Transcript {
        role: "Runtime",
        message: "Received OAuth callback, exchanging token.".into(),
    });
    let token = oauth_manager.exchange_code(&code, &verifier, port).await?;
    Ok(token.access_token)
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
    );

    Ok(Agent::new(
        tool_manager,
        backend_arc,
        vdb,
        session_manager,
        workspace,
    ))
}

fn create_full_tool_manager(
    backend: Arc<dyn LlmBackend>,
    vdb: Arc<VectorDB>,
    session_manager: Arc<SessionManager>,
    workspace: Arc<WorkspaceMemory>,
    sandbox: Arc<SandboxManager>,
    skill_manager: Arc<SkillManager>,
) -> ToolManager {
    let mut tm = ToolManager::new();
    tm.register(Box::new(BashTool {
        sandbox: sandbox.clone(),
    }));
    tm.register(Box::new(ReadFileTool));
    tm.register(Box::new(WriteFileTool));
    tm.register(Box::new(ListFilesTool));
    tm.register(Box::new(SearchFilesTool));
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
    }));
    tm.register(Box::new(TeamCreateTool {
        backend,
        vdb,
        session_manager,
        workspace,
    }));
    tm
}
