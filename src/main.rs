mod acp;
mod agent;
mod config;
mod llm;
mod local_backend;
mod oauth;
mod prompt;
mod redaction;
mod sandbox;
mod session;
mod skill;
mod state_db;
mod tool;
mod tool_result;
mod tools;
mod tui;
mod vectordb;
mod workspace;

use crate::acp::{run_acp_stdio, RaraAcpAgent};
use crate::agent::Agent;
use crate::config::{ConfigManager, RaraConfig};
use crate::llm::{
    CodexBackend, GeminiBackend, LlmBackend, MockLlm, OllamaBackend, OpenAiCompatibleBackend,
};
use crate::local_backend::{LocalLlmBackend, LocalProgressReporter};
use crate::oauth::OAuthManager;
use crate::redaction::redact_secrets;
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
use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use secrecy::ExposeSecret;
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
    Ask {
        prompt: String,
    },
    Login {
        #[arg(long)]
        device_auth: bool,
        #[arg(long)]
        with_api_key: bool,
    },
    Logout,
    Tui,
}

#[tokio::main]
async fn main() {
    if let Err(err) = main_impl().await {
        eprintln!("{}", redact_secrets(format!("Error: {err}")));
        std::process::exit(1);
    }
}

async fn main_impl() -> Result<()> {
    let cli = Cli::parse();
    let config_manager = ConfigManager::new()?;
    let mut config = config_manager.load()?;

    if let Some(p) = cli.provider {
        config.set_provider(p);
    }
    if let Some(k) = cli.api_key {
        config.set_api_key(k);
    }
    if let Some(b) = cli.base_url {
        config.set_base_url(Some(b));
    }
    if let Some(m) = cli.model {
        config.set_model(Some(m));
    }
    if let Some(r) = cli.revision {
        config.set_revision(Some(r));
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
        prompt::PromptRuntimeConfig::from_config(&config),
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
            agent.set_prompt_config(prompt::PromptRuntimeConfig::from_config(&config));
            agent.query(prompt).await?;
        }
        Commands::Login {
            device_auth,
            with_api_key,
        } => {
            if device_auth && with_api_key {
                bail!("choose either --device-auth or --with-api-key, not both");
            }
            if with_api_key {
                let oauth_reader = oauth_manager.clone();
                let api_key =
                    tokio::task::spawn_blocking(move || oauth_reader.read_api_key_from_stdin())
                        .await??;
                let credential = oauth_manager.save_api_key(api_key.expose_secret())?;
                save_codex_credential(
                    &mut config,
                    &config_manager,
                    credential.expose_secret(),
                )?;
                println!("Successfully saved Codex API key.");
            } else if device_auth {
                let token = oauth_manager.request_device_code().await?;
                eprintln!(
                    "Open this URL and enter the one-time code:\n{}\n\nCode: {}",
                    token.verification_url, token.user_code
                );
                let credential = oauth_manager.complete_device_code_login(&token).await?;
                save_codex_credential(
                    &mut config,
                    &config_manager,
                    credential.expose_secret(),
                )?;
                println!("Successfully logged in with device code.");
            } else {
                if std::env::var_os("SSH_CONNECTION").is_some() {
                    bail!("browser login is not reliable in SSH/headless sessions; use --device-auth or --with-api-key");
                }
                let session = oauth_manager.start_browser_login(true)?;
                eprintln!(
                    "Starting local login flow.\nIf your browser did not open, navigate to this URL:\n\n{}",
                    session.auth_url()
                );
                let credential = session.complete(&oauth_manager).await?;
                save_codex_credential(
                    &mut config,
                    &config_manager,
                    credential.expose_secret(),
                )?;
                println!("Successfully logged in.");
            }
        }
        Commands::Logout => {
            let removed = oauth_manager.clear_saved_auth()?;
            config.clear_provider_api_key("codex");
            config_manager.save(&config)?;
            if removed {
                println!("Removed the saved Codex credential.");
            } else {
                println!("No saved Codex credential was present.");
            }
        }
        Commands::Tui => {
            let mut agent = Agent::new(tool_manager, backend_arc, vdb, session_manager, workspace);
            agent.set_prompt_config(prompt::PromptRuntimeConfig::from_config(&config));
            crate::tui::run_tui(agent, oauth_manager).await?;
        }
    }
    Ok(())
}

fn save_codex_credential(
    config: &mut RaraConfig,
    config_manager: &ConfigManager,
    credential: &str,
) -> Result<()> {
    config.set_provider("codex");
    config.set_api_key(credential.to_string());
    if config.model.is_none() {
        config.set_model(Some("codex".into()));
    }
    config_manager.save(config)
}

fn create_full_tool_manager(
    backend: Arc<dyn LlmBackend>,
    vdb: Arc<VectorDB>,
    session_manager: Arc<SessionManager>,
    workspace: Arc<WorkspaceMemory>,
    sandbox: Arc<SandboxManager>,
    skill_manager: Arc<SkillManager>,
    prompt_config: prompt::PromptRuntimeConfig,
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
        "kimi" => Ok(Box::new(OpenAiCompatibleBackend::new(
            Some(
                config
                    .api_key
                    .clone()
                    .context("API key required for Kimi provider")?,
            ),
            "https://api.moonshot.cn/v1".to_string(),
            config
                .model
                .clone()
                .unwrap_or_else(|| "moonshot-v1-8k".to_string()),
        )?)),
        "codex" => Ok(Box::new(CodexBackend::new(
            config.api_key.clone(),
            config
                .base_url
                .clone()
                .unwrap_or_else(|| "http://localhost:8080".to_string()),
            config.model.clone().unwrap_or_else(|| "codex".to_string()),
        )?)),
        "ollama" | "ollama-native" => Ok(Box::new(OllamaBackend::new(
            config
                .base_url
                .clone()
                .unwrap_or_else(|| "http://localhost:11434".to_string()),
            config.model.clone().unwrap_or_else(|| "gemma4".to_string()),
            config.thinking.unwrap_or(true),
            config.num_ctx,
        )?)),
        "ollama-openai" => Ok(Box::new(OpenAiCompatibleBackend::new(
            config.api_key.clone(),
            config
                .base_url
                .clone()
                .unwrap_or_else(|| "http://localhost:11434".to_string()),
            config.model.clone().unwrap_or_else(|| "gemma4".to_string()),
        )?)),
        "gemini" => Ok(Box::new(GeminiBackend {
            api_key: config
                .api_key
                .clone()
                .context("API key required for Gemini provider")?,
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
        other => bail!("Unsupported provider '{other}'"),
    }
}

#[cfg(test)]
mod tests {
    use super::build_backend_with_progress;
    use crate::config::RaraConfig;

    #[tokio::test]
    async fn unsupported_provider_returns_error() {
        let config = RaraConfig {
            provider: "does-not-exist".to_string(),
            ..Default::default()
        };

        let err = match build_backend_with_progress(&config, None).await {
            Ok(_) => panic!("unsupported provider should fail"),
            Err(err) => err,
        };

        assert!(err
            .to_string()
            .contains("Unsupported provider 'does-not-exist'"));
    }
}
