mod acp;
mod agent;
mod codex_model_catalog;
mod config;
mod context;
mod llm;
mod local_backend;
mod oauth;
mod prompt;
mod redaction;
mod runtime_context;
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
use crate::config::{
    ConfigManager, RaraConfig, DEFAULT_CODEX_BASE_URL, DEFAULT_CODEX_CHATGPT_BASE_URL,
};
use crate::llm::{LlmBackend, MockLlm};
use crate::oauth::{OAuthManager, SavedCodexAuthMode};
use crate::redaction::redact_secrets;
use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use secrecy::ExposeSecret;

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

#[derive(Subcommand, Debug)]
enum Commands {
    Acp,
    Ask {
        prompt: String,
    },
    Resume {
        session_id: Option<String>,
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

    match cli.command.unwrap_or(Commands::Tui) {
        Commands::Acp => run_acp_command(&config).await?,
        Commands::Ask { prompt } => run_ask_command(&config, prompt).await?,
        Commands::Resume { session_id } => {
            run_tui_command(&config, oauth_manager, session_id).await?
        }
        Commands::Login {
            device_auth,
            with_api_key,
        } => run_login_command(
            &mut config,
            &config_manager,
            &oauth_manager,
            device_auth,
            with_api_key,
        )
        .await?,
        Commands::Logout => run_logout_command(&mut config, &config_manager, &oauth_manager)?,
        Commands::Tui => run_tui_command(&config, oauth_manager, None).await?,
    }
    Ok(())
}

async fn run_acp_command(config: &RaraConfig) -> Result<()> {
    let bootstrap = runtime_context::initialize_rara_context(config, None).await?;
    emit_bootstrap_warnings(&bootstrap.warnings);
    let backend_builder = Box::new(move || Box::new(MockLlm) as Box<dyn LlmBackend>);
    let acp_agent = RaraAcpAgent {
        tool_manager: bootstrap.tool_manager,
        backend_builder,
    };
    run_acp_stdio(acp_agent).await
}

async fn run_ask_command(config: &RaraConfig, prompt: String) -> Result<()> {
    let bootstrap = runtime_context::initialize_rara_context(config, None).await?;
    emit_bootstrap_warnings(&bootstrap.warnings);
    let mut agent = bootstrap.into_agent();
    agent.query(prompt).await
}

async fn run_tui_command(
    config: &RaraConfig,
    oauth_manager: OAuthManager,
    resume_session_id: Option<String>,
) -> Result<()> {
    let bootstrap = runtime_context::initialize_rara_context(config, None).await?;
    emit_bootstrap_warnings(&bootstrap.warnings);
    let agent = bootstrap.into_agent();
    let resumed_session_id = crate::tui::run_tui(agent, oauth_manager, resume_session_id).await?;
    if let Some(session_id) = resumed_session_id {
        println!("{}", resume_hint(&session_id));
    }
    Ok(())
}

fn resume_hint(session_id: &str) -> String {
    format!("Resume this session with: rara resume {session_id}")
}

async fn run_login_command(
    config: &mut RaraConfig,
    config_manager: &ConfigManager,
    oauth_manager: &OAuthManager,
    device_auth: bool,
    with_api_key: bool,
) -> Result<()> {
    if device_auth && with_api_key {
        bail!("choose either --device-auth or --with-api-key, not both");
    }
    if with_api_key {
        let oauth_reader = oauth_manager.clone();
        let api_key = tokio::task::spawn_blocking(move || oauth_reader.read_api_key_from_stdin())
            .await??;
        let credential = oauth_manager.save_api_key(api_key.expose_secret())?;
        save_codex_credential(
            config,
            config_manager,
            oauth_manager,
            credential.expose_secret(),
        )?;
        println!("Successfully saved Codex API key.");
        return Ok(());
    }
    if device_auth {
        let token = oauth_manager.request_device_code().await?;
        eprintln!(
            "Open this URL and enter the one-time code:\n{}\n\nCode: {}",
            token.verification_url, token.user_code
        );
        let credential = oauth_manager.complete_device_code_login(&token).await?;
        save_codex_credential(
            config,
            config_manager,
            oauth_manager,
            credential.expose_secret(),
        )?;
        println!("Successfully logged in with device code.");
        return Ok(());
    }

    if std::env::var_os("SSH_CONNECTION").is_some() {
        bail!("browser login is not reliable in SSH/headless sessions; use --device-auth or --with-api-key");
    }
    let session = oauth_manager.start_browser_login(true)?;
    eprintln!(
        "Starting local login flow.\nIf your browser did not open, navigate to this URL:\n\n{}",
        session.auth_url()
    );
    let credential = session.complete(oauth_manager).await?;
    save_codex_credential(
        config,
        config_manager,
        oauth_manager,
        credential.expose_secret(),
    )?;
    println!("Successfully logged in.");
    Ok(())
}

fn run_logout_command(
    config: &mut RaraConfig,
    config_manager: &ConfigManager,
    oauth_manager: &OAuthManager,
) -> Result<()> {
    let removed = oauth_manager.clear_saved_auth()?;
    config.clear_provider_api_key("codex");
    config_manager.save(config)?;
    if removed {
        println!("Removed the saved Codex credential.");
    } else {
        println!("No saved Codex credential was present.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_hint_includes_exact_command() {
        assert_eq!(
            resume_hint("session-123"),
            "Resume this session with: rara resume session-123"
        );
    }

    #[test]
    fn clap_parses_resume_command_with_optional_session_id() {
        let cli = Cli::try_parse_from(["rara", "resume", "session-123"]).expect("parse resume");
        match cli.command.expect("command") {
            Commands::Resume { session_id } => {
                assert_eq!(session_id.as_deref(), Some("session-123"));
            }
            other => panic!("unexpected command: {other:?}"),
        }

        let cli = Cli::try_parse_from(["rara", "resume"]).expect("parse resume without id");
        match cli.command.expect("command") {
            Commands::Resume { session_id } => {
                assert_eq!(session_id, None);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }
}

fn emit_bootstrap_warnings(warnings: &[String]) {
    for warning in warnings {
        eprintln!("{}", redact_secrets(format!("Warning: {warning}")));
    }
}

fn save_codex_credential(
    config: &mut RaraConfig,
    config_manager: &ConfigManager,
    oauth_manager: &OAuthManager,
    credential: &str,
) -> Result<()> {
    config.set_provider("codex");
    config.set_api_key(credential.to_string());
    let base_url = match oauth_manager.saved_auth_mode()? {
        Some(SavedCodexAuthMode::Chatgpt) => DEFAULT_CODEX_CHATGPT_BASE_URL,
        _ => DEFAULT_CODEX_BASE_URL,
    };
    config.apply_codex_defaults_for_base_url(base_url);
    config_manager.save(config)
}
