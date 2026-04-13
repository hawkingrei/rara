mod acp;
mod agent;
mod config;
mod local_backend;
mod llm;
mod oauth;
mod sandbox;
mod session;
mod skill;
mod tool;
mod tools;
mod tui;
mod vectordb;
mod workspace;

use crate::acp::{run_acp_stdio, RaraAcpAgent};
use crate::agent::Agent;
use crate::config::{ConfigManager, RaraConfig};
use crate::llm::{CodexBackend, GeminiBackend, LlmBackend, MockLlm, OpenAiCompatibleBackend};
use crate::local_backend::{LocalLlmBackend, LocalProgressReporter};
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
use anyhow::Result;
use clap::{Parser, Subcommand};
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "rara")]
#[command(about = "RARA: RARA Automates Rust Agents", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    #[arg(short, long, global = true)]
    provider: Option<String>,

    #[arg(short, long, env = "RARA_API_KEY", global = true)]
    api_key: Option<String>,

    #[arg(short, long, global = true)]
    base_url: Option<String>,

    #[arg(short, long, global = true)]
    model: Option<String>,

    #[arg(long, global = true)]
    revision: Option<String>,
}

#[derive(Subcommand)]
enum Commands {
    Acp,
    Ask { prompt: String },
    Tui,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config_manager = ConfigManager::new()?;
    let mut config = config_manager.load();

    if let Some(p) = cli.provider {
        config.provider = p;
    }
    if let Some(k) = cli.api_key {
        config.api_key = Some(k);
    }
    if let Some(b) = cli.base_url {
        config.base_url = Some(b);
    }
    if let Some(m) = cli.model {
        config.model = Some(m);
    }
    if let Some(r) = cli.revision {
        config.revision = Some(r);
    }

    let oauth_manager = OAuthManager::new()?;
    let vdb = Arc::new(VectorDB::new("data/lancedb"));
    let session_manager = Arc::new(SessionManager::new()?);
    let workspace = Arc::new(WorkspaceMemory::new()?);
    let sandbox_manager = Arc::new(SandboxManager::new()?);

    let mut skill_manager = SkillManager::new();
    let _ = skill_manager.load_all();
    let skill_manager_arc = Arc::new(skill_manager);

    let backend = build_backend(&config).await?;
    let backend_arc: Arc<dyn LlmBackend> = backend.into();

    let tool_manager = create_full_tool_manager(
        backend_arc.clone(),
        vdb.clone(),
        session_manager.clone(),
        workspace.clone(),
        sandbox_manager.clone(),
        skill_manager_arc.clone(),
    );

    match cli.command.unwrap_or(Commands::Tui) {
        Commands::Acp => {
            let backend_builder = Box::new(move || Box::new(MockLlm) as Box<dyn LlmBackend>);
            let acp_agent = RaraAcpAgent {
                tool_manager,
                backend_builder,
            };
            run_acp_stdio(acp_agent).await?;
        }
        Commands::Ask { prompt } => {
            let mut agent = Agent::new(tool_manager, backend_arc, vdb, session_manager, workspace);
            agent.query(prompt).await?;
        }
        Commands::Tui => {
            let agent = Agent::new(tool_manager, backend_arc, vdb, session_manager, workspace);
            crate::tui::run_tui(agent, oauth_manager).await?;
        }
    }
    Ok(())
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
        backend: backend.clone(),
        vdb: vdb.clone(),
        session_manager: session_manager.clone(),
        workspace: workspace.clone(),
    }));
    tm
}

pub(crate) async fn build_backend(config: &RaraConfig) -> Result<Box<dyn LlmBackend>> {
    build_backend_with_progress(config, None).await
}

pub(crate) async fn build_backend_with_progress(
    config: &RaraConfig,
    progress: Option<LocalProgressReporter>,
) -> Result<Box<dyn LlmBackend>> {
    match config.provider.as_str() {
        "kimi" => Ok(Box::new(OpenAiCompatibleBackend {
            api_key: config.api_key.clone().expect("API key required for Kimi"),
            base_url: "https://api.moonshot.cn/v1".to_string(),
            model: config
                .model
                .clone()
                .unwrap_or_else(|| "moonshot-v1-8k".to_string()),
        })),
        "codex" => Ok(Box::new(CodexBackend {
            api_key: config.api_key.clone().unwrap_or_default(),
            base_url: config
                .base_url
                .clone()
                .unwrap_or_else(|| "http://localhost:8080".to_string()),
            model: config.model.clone().unwrap_or_else(|| "codex".to_string()),
        })),
        "ollama" => Ok(Box::new(OpenAiCompatibleBackend {
            api_key: config.api_key.clone().unwrap_or_default(),
            base_url: config
                .base_url
                .clone()
                .unwrap_or_else(|| "http://localhost:11434/v1".to_string()),
            model: config.model.clone().unwrap_or_else(|| "gemma4:e4b".to_string()),
        })),
        "gemini" => Ok(Box::new(GeminiBackend {
            api_key: config.api_key.clone().expect("API key required for Gemini"),
            model: config
                .model
                .clone()
                .unwrap_or_else(|| "gemini-1.5-pro".to_string()),
        })),
        "gemma4" | "qwen3" | "qwn3" | "local" | "local-candle" => {
            let config = config.clone();
            let progress = progress.clone();
            let backend = tokio::task::spawn_blocking(move || {
                LocalLlmBackend::from_config_with_progress(&config, progress)
            })
            .await??;
            Ok(Box::new(backend))
        }
        "mock" => Ok(Box::new(MockLlm)),
        _ => Ok(Box::new(MockLlm)),
    }
}
