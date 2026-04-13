use std::sync::Arc;
use std::time::Instant;

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

use super::state::{
    HelpTab, LocalCommand, LocalCommandKind, Overlay, RunningTask, RuntimePhase, TaskCompletion,
    TaskKind, TuiApp, TuiEvent,
};

pub async fn execute_local_command(
    command: LocalCommand,
    app: &mut TuiApp,
    agent_slot: &mut Option<Agent>,
    oauth_manager: &Arc<OAuthManager>,
) -> anyhow::Result<bool> {
    app.remember_command(match command.kind {
        LocalCommandKind::Help => "help",
        LocalCommandKind::Status => "status",
        LocalCommandKind::Clear => "clear",
        LocalCommandKind::Setup => "setup",
        LocalCommandKind::Model => "model",
        LocalCommandKind::BaseUrl => "base-url",
        LocalCommandKind::Login => "login",
        LocalCommandKind::Quit => "quit",
    });
    match command.kind {
        LocalCommandKind::Help => {
            app.set_runtime_phase(RuntimePhase::LocalCommand, Some("opening help".into()));
            app.open_overlay(Overlay::Help(HelpTab::General));
        }
        LocalCommandKind::Status => {
            app.set_runtime_phase(RuntimePhase::LocalCommand, Some("opening status".into()));
            app.open_overlay(Overlay::Status);
        }
        LocalCommandKind::Clear => {
            app.set_runtime_phase(RuntimePhase::LocalCommand, Some("clearing transcript".into()));
            app.reset_transcript();
        }
        LocalCommandKind::Setup => {
            app.set_runtime_phase(RuntimePhase::LocalCommand, Some("opening setup".into()));
            app.open_overlay(Overlay::Setup);
        }
        LocalCommandKind::Model => handle_model_command(command.arg.as_deref(), app)?,
        LocalCommandKind::BaseUrl => handle_base_url_command(command.arg.as_deref(), app)?,
        LocalCommandKind::Login => {
            if app.is_busy() {
                app.push_notice("A task is already running. Wait for it to finish.");
            } else {
                start_oauth_task(app, Arc::clone(oauth_manager));
            }
        }
        LocalCommandKind::Quit => {
            app.set_runtime_phase(RuntimePhase::LocalCommand, Some("quitting".into()));
            return Ok(true);
        }
    }
    if let Some(agent) = agent_slot.as_ref() {
        app.sync_snapshot(agent);
    }
    Ok(false)
}

pub fn start_query_task(app: &mut TuiApp, prompt: String, mut agent: Agent) {
    let (sender, receiver) = mpsc::unbounded_channel();
    app.notice = Some("Running prompt.".into());
    app.set_runtime_phase(RuntimePhase::SendingPrompt, Some("sending prompt".into()));
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
        started_at: Instant::now(),
        next_heartbeat_after_secs: 2,
    });
}

pub fn start_rebuild_task(app: &mut TuiApp) {
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

pub fn start_oauth_task(app: &mut TuiApp, oauth_manager: Arc<OAuthManager>) {
    let (sender, receiver) = mpsc::unbounded_channel();
    app.notice = Some("Starting OAuth login.".into());
    app.set_runtime_phase(RuntimePhase::OAuthStarting, Some("starting oauth".into()));
    app.push_entry("Runtime", "Starting OAuth login flow.");

    let handle = tokio::spawn(async move {
        let result = run_oauth_login(oauth_manager, sender.clone()).await;
        TaskCompletion::OAuth { result }
    });

    app.running_task = Some(RunningTask {
        kind: TaskKind::OAuth,
        receiver,
        handle,
        started_at: Instant::now(),
        next_heartbeat_after_secs: u64::MAX,
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
            *agent_slot = Some(agent);
            if let Some(agent) = agent_slot.as_ref() {
                app.sync_snapshot(agent);
            }
            match result {
                Ok(_) => {
                    app.notice = Some("Prompt finished.".into());
                    app.set_runtime_phase(RuntimePhase::Idle, Some("prompt finished".into()));
                }
                Err(err) => {
                    app.set_runtime_phase(RuntimePhase::Failed, Some("query failed".into()));
                    let mut message = format!("Query failed: {err}");
                    if app.config.provider == "ollama" {
                        let base_url = app
                            .config
                            .base_url
                            .as_deref()
                            .unwrap_or("http://localhost:11434");
                        message.push_str(&format!("\nbase_url={base_url}"));
                    }
                    app.push_notice(message);
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
                app.set_runtime_phase(RuntimePhase::BackendReady, Some("backend ready".into()));
                app.push_entry("Runtime", app.setup_status.clone().unwrap_or_default());
            }
            Err(err) => {
                app.set_runtime_phase(
                    RuntimePhase::Failed,
                    Some("backend rebuild failed".into()),
                );
                let message = format!("Failed to apply config:\n{}", format_error_chain(&err));
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
                app.set_runtime_phase(RuntimePhase::OAuthSaved, Some("oauth token saved".into()));
                app.close_overlay();
                app.push_entry("Runtime", "Saved OAuth token.");
            }
            Err(err) => {
                app.set_runtime_phase(RuntimePhase::Failed, Some("oauth failed".into()));
                app.push_notice(format!("OAuth failed:\n{}", format_error_chain(&err)));
            }
        },
    }

    Ok(())
}

fn emit_query_heartbeat(app: &mut TuiApp) {
    let Some(task) = app.running_task.as_mut() else {
        return;
    };
    if !matches!(task.kind, TaskKind::Query) || !super::command::is_local_provider(&app.config.provider) {
        return;
    }

    let elapsed = task.started_at.elapsed().as_secs();
    if elapsed < task.next_heartbeat_after_secs {
        return;
    }

    let detail = format!("local model is still generating · {}s elapsed", elapsed);
    task.next_heartbeat_after_secs = elapsed.saturating_add(1);

    app.set_runtime_phase(RuntimePhase::SendingPrompt, Some(detail.clone()));
    app.notice = Some(format!("Working locally · {}s elapsed", elapsed));
}

fn handle_model_command(arg: Option<&str>, app: &mut TuiApp) -> anyhow::Result<()> {
    if arg.map(str::trim).filter(|arg| !arg.is_empty()).is_some() {
        app.push_notice("/model does not accept arguments. Use the interactive menu.");
    }
    app.open_overlay(Overlay::ModelGuide);
    app.notice = Some("Opened model guide.".into());
    Ok(())
}

fn handle_base_url_command(arg: Option<&str>, app: &mut TuiApp) -> anyhow::Result<()> {
    if arg.map(str::trim).filter(|arg| !arg.is_empty()).is_some() {
        app.push_notice("/base-url does not accept arguments. Edit the value in the TUI.");
    }
    app.open_overlay(Overlay::BaseUrlEditor);
    Ok(())
}

fn apply_tui_event(app: &mut TuiApp, event: TuiEvent) {
    match event {
        TuiEvent::Transcript { role, message } => {
            if role == "Status" {
                app.set_runtime_phase(
                    RuntimePhase::ProcessingResponse,
                    Some(message.lines().next().unwrap_or(role).trim().to_string()),
                );
            } else if role == "Tool" || role == "Tool Result" || role == "Tool Error" {
                app.set_runtime_phase(
                    RuntimePhase::RunningTool,
                    Some(message.lines().next().unwrap_or(role).trim().to_string()),
                );
            } else if role == "Agent" {
                app.set_runtime_phase(
                    RuntimePhase::ProcessingResponse,
                    Some("receiving model output".into()),
                );
            } else if role == "Download" {
                let detail = message.lines().next().unwrap_or(role).trim().to_string();
                if detail.starts_with("Ready ·") {
                    app.set_runtime_phase(RuntimePhase::BackendReady, Some(detail));
                } else {
                    app.set_runtime_phase(RuntimePhase::RebuildingBackend, Some(detail));
                }
            } else if role == "Runtime" {
                let detail = message.lines().next().unwrap_or(role).trim().to_string();
                if detail.contains("OAuth flow") {
                    app.set_runtime_phase(RuntimePhase::OAuthWaitingCallback, Some(detail));
                } else if detail.contains("exchanging token") {
                    app.set_runtime_phase(RuntimePhase::OAuthExchangingToken, Some(detail));
                } else {
                    app.set_runtime_phase(RuntimePhase::RebuildingBackend, Some(detail));
                }
            }
            app.push_entry(role, message)
        }
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
            message: format_tool_use(&name, &input),
        },
        AgentEvent::ToolResult {
            name,
            content,
            is_error,
        } => TuiEvent::Transcript {
            role: if is_error { "Tool Error" } else { "Tool Result" },
            message: format_tool_result(&name, &content),
        },
    }
}

fn format_tool_use(name: &str, input: &serde_json::Value) -> String {
    match name {
        "bash" => input
            .get("command")
            .and_then(serde_json::Value::as_str)
            .map(|command| format!("bash {command}"))
            .unwrap_or_else(|| format!("{name} {input}")),
        "read_file" => input
            .get("path")
            .and_then(serde_json::Value::as_str)
            .map(|path| format!("read_file {path}"))
            .unwrap_or_else(|| format!("{name} {input}")),
        "write_file" => input
            .get("path")
            .and_then(serde_json::Value::as_str)
            .map(|path| format!("write_file {path}"))
            .unwrap_or_else(|| format!("{name} {input}")),
        "replace" => input
            .get("path")
            .and_then(serde_json::Value::as_str)
            .map(|path| format!("replace {path}"))
            .unwrap_or_else(|| format!("{name} {input}")),
        "list_files" => input
            .get("path")
            .and_then(serde_json::Value::as_str)
            .map(|path| format!("list_files {path}"))
            .unwrap_or_else(|| format!("{name} {input}")),
        "grep" => {
            let pattern = input
                .get("pattern")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("<pattern>");
            let path = input
                .get("path")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(".");
            format!("grep {pattern} in {path}")
        }
        "glob" => {
            let pattern = input
                .get("pattern")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("<pattern>");
            let path = input
                .get("path")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(".");
            format!("glob {pattern} in {path}")
        }
        "web_fetch" => input
            .get("url")
            .and_then(serde_json::Value::as_str)
            .map(|url| format!("web_fetch {url}"))
            .unwrap_or_else(|| format!("{name} {input}")),
        "apply_patch" => "apply_patch".to_string(),
        _ => format!("{name} {input}"),
    }
}

fn format_tool_result(name: &str, content: &str) -> String {
    if name == "bash" {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(content) {
            let exit_code = value
                .get("exit_code")
                .and_then(serde_json::Value::as_i64)
                .map(|code| code.to_string())
                .unwrap_or_else(|| "?".to_string());
            let stdout = value
                .get("stdout")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let stderr = value
                .get("stderr")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let mut summary = format!("bash exit_code={exit_code}");
            if !stdout.trim().is_empty() {
                summary.push_str(&format!("\nstdout: {}", first_non_empty_line(stdout)));
            }
            if !stderr.trim().is_empty() {
                summary.push_str(&format!("\nstderr: {}", first_non_empty_line(stderr)));
            }
            return summary;
        }
    }

    if name == "list_files" {
        return content.to_string();
    }

    if let Some(summary) = content.lines().next().map(str::trim).filter(|line| !line.is_empty()) {
        let mut rendered = format!("{name}: {summary}");
        if content.contains("full_result_path=") {
            rendered.push_str("\nfull result stored on disk");
        } else if content.lines().nth(1).is_some() {
            rendered.push_str("\npreview available");
        }
        return rendered;
    }

    format!("{name}: {content}")
}

fn first_non_empty_line(text: &str) -> &str {
    text.lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or(text)
}

fn format_error_chain(err: &anyhow::Error) -> String {
    let mut lines = Vec::new();
    for (idx, cause) in err.chain().enumerate() {
        if idx == 0 {
            lines.push(cause.to_string());
        } else {
            lines.push(format!("caused by: {cause}"));
        }
    }
    lines.join("\n")
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
