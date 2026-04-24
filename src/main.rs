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
mod thread_cli;
mod thread_store;
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
use crate::tui::StartupResumeTarget;

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

#[derive(Debug, Subcommand)]
enum Commands {
    Acp,
    Ask {
        prompt: String,
    },
    Thread {
        #[arg(value_name = "THREAD_ID")]
        thread_id: String,
    },
    Threads {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    Resume {
        #[arg(value_name = "THREAD_ID")]
        thread_id: Option<String>,
        #[arg(long, conflicts_with = "thread_id")]
        last: bool,
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
    let command = cli.command.unwrap_or(Commands::Tui);

    match command {
        Commands::Acp => run_acp_command(&config).await?,
        Commands::Ask { prompt } => run_ask_command(&config, prompt).await?,
        Commands::Thread { thread_id } => thread_cli::run_thread_command(&thread_id)?,
        Commands::Threads { limit } => thread_cli::run_threads_command(limit)?,
        Commands::Resume { thread_id, last } => {
            let startup_resume = startup_resume_target_for_command(&Commands::Resume {
                thread_id,
                last,
            })
            .expect("resume command should always map to a startup target");
            run_tui_command(&config, oauth_manager, startup_resume).await?
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
        Commands::Tui => {
            let startup_resume = startup_resume_target_for_command(&Commands::Tui)
                .expect("tui command should always map to a startup target");
            run_tui_command(&config, oauth_manager, startup_resume).await?
        }
    }
    Ok(())
}

fn startup_resume_target_for_command(command: &Commands) -> Option<StartupResumeTarget> {
    match command {
        Commands::Resume {
            thread_id: Some(thread_id),
            ..
        } => Some(StartupResumeTarget::ThreadId(thread_id.clone())),
        Commands::Resume {
            thread_id: None,
            last: true,
        } => Some(StartupResumeTarget::Latest),
        Commands::Resume {
            thread_id: None,
            last: false,
        } => Some(StartupResumeTarget::Picker),
        Commands::Tui => Some(StartupResumeTarget::Fresh),
        Commands::Acp
        | Commands::Ask { .. }
        | Commands::Thread { .. }
        | Commands::Threads { .. }
        | Commands::Login { .. }
        | Commands::Logout => None,
    }
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
    startup_resume: StartupResumeTarget,
) -> Result<()> {
    let bootstrap = runtime_context::initialize_rara_context(config, None).await?;
    emit_bootstrap_warnings(&bootstrap.warnings);
    let agent = bootstrap.into_agent();
    crate::tui::run_tui(agent, oauth_manager, startup_resume).await
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

#[cfg(test)]
mod tests {
    use super::{startup_resume_target_for_command, Cli, Commands};
    use crate::tui::StartupResumeTarget;
    use clap::Parser;

    #[test]
    fn clap_parses_resume_command_with_optional_thread_id() {
        let cli = Cli::parse_from(["rara", "resume", "thread-123"]);
        match cli.command.expect("command") {
            Commands::Resume { thread_id, last } => {
                assert_eq!(thread_id.as_deref(), Some("thread-123"));
                assert!(!last);
            }
            other => panic!("expected resume command, got {other:?}"),
        }

        let cli = Cli::parse_from(["rara", "resume"]);
        match cli.command.expect("command") {
            Commands::Resume { thread_id, last } => {
                assert!(thread_id.is_none());
                assert!(!last);
            }
            other => panic!("expected resume command, got {other:?}"),
        }

        let cli = Cli::parse_from(["rara", "resume", "--last"]);
        match cli.command.expect("command") {
            Commands::Resume { thread_id, last } => {
                assert!(thread_id.is_none());
                assert!(last);
            }
            other => panic!("expected resume command, got {other:?}"),
        }
    }

    #[test]
    fn clap_parses_thread_lifecycle_commands() {
        let cli = Cli::parse_from(["rara", "threads", "--limit", "5"]);
        match cli.command {
            Some(Commands::Threads { limit }) => assert_eq!(limit, 5),
            other => panic!("expected threads command, got {other:?}"),
        }

        let cli = Cli::parse_from(["rara", "thread", "thread-123"]);
        match cli.command {
            Some(Commands::Thread { thread_id }) => assert_eq!(thread_id, "thread-123"),
            other => panic!("expected thread command, got {other:?}"),
        }
    }

    #[test]
    fn startup_resume_targets_are_explicit() {
        assert!(matches!(
            startup_resume_target_for_command(&Commands::Tui),
            Some(StartupResumeTarget::Fresh)
        ));
        assert!(matches!(
            startup_resume_target_for_command(&Commands::Resume {
                thread_id: None,
                last: false,
            }),
            Some(StartupResumeTarget::Picker)
        ));
        assert!(matches!(
            startup_resume_target_for_command(&Commands::Resume {
                thread_id: None,
                last: true,
            }),
            Some(StartupResumeTarget::Latest)
        ));
        assert!(matches!(
            startup_resume_target_for_command(&Commands::Resume {
                thread_id: Some("thread-123".to_string()),
                last: false,
            }),
            Some(StartupResumeTarget::ThreadId(thread_id)) if thread_id == "thread-123"
        ));
    }
}
