use crate::sandbox::SandboxManager;
use crate::tool::{Tool, ToolError, ToolOutputStream, ToolProgressEvent};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::env;
use std::process::Stdio;
use std::sync::Arc;
use tokio::fs;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio::sync::mpsc;

pub struct BashTool {
    pub sandbox: Arc<SandboxManager>,
}

const SANDBOX_HOME: &str = "/tmp/rara-home";

#[derive(Clone, Copy, Debug)]
enum BashStreamKind {
    Stdout,
    Stderr,
}

impl BashStreamKind {
    fn output_stream(self) -> ToolOutputStream {
        match self {
            Self::Stdout => ToolOutputStream::Stdout,
            Self::Stderr => ToolOutputStream::Stderr,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BashCommandInput {
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub program: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub allow_net: bool,
}

impl BashCommandInput {
    pub fn from_value(input: Value) -> Result<Self, ToolError> {
        let parsed: Self = serde_json::from_value(input)
            .map_err(|err| ToolError::InvalidInput(format!("bash payload: {err}")))?;
        parsed.validate()?;
        Ok(parsed)
    }

    pub fn validate(&self) -> Result<(), ToolError> {
        let has_command = self
            .command
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty());
        let has_program = self
            .program
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty());
        if !has_command && !has_program {
            return Err(ToolError::InvalidInput(
                "bash payload requires either command or program".into(),
            ));
        }
        Ok(())
    }

    pub fn working_dir(&self) -> Result<String, ToolError> {
        match self.cwd.as_ref() {
            Some(cwd) if !cwd.trim().is_empty() => Ok(cwd.clone()),
            _ => Ok(env::current_dir()?.to_string_lossy().to_string()),
        }
    }

    pub fn summary(&self) -> String {
        if let Some(command) = self
            .command
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            return command.to_string();
        }

        let program = self
            .program
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("<program>");
        if self.args.is_empty() {
            program.to_string()
        } else {
            format!("{program} {}", self.args.join(" "))
        }
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).expect("bash command input should serialize")
    }
}

fn sandbox_command_env(overrides: &HashMap<String, String>) -> HashMap<String, String> {
    let mut env_map = HashMap::from([
        ("HOME".to_string(), SANDBOX_HOME.to_string()),
        (
            "XDG_CONFIG_HOME".to_string(),
            format!("{SANDBOX_HOME}/.config"),
        ),
        (
            "XDG_CACHE_HOME".to_string(),
            format!("{SANDBOX_HOME}/.cache"),
        ),
        (
            "XDG_STATE_HOME".to_string(),
            format!("{SANDBOX_HOME}/.local/state"),
        ),
        (
            "XDG_DATA_HOME".to_string(),
            format!("{SANDBOX_HOME}/.local/share"),
        ),
    ]);
    env_map.extend(overrides.clone());
    env_map
}

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }
    fn description(&self) -> &str {
        "Run shell command in sandbox"
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Legacy shell command string. Prefer program+args for new calls."
                },
                "program": {
                    "type": "string",
                    "description": "Executable to run directly without a shell."
                },
                "args": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Arguments for program."
                },
                "cwd": {
                    "type": "string",
                    "description": "Optional working directory override."
                },
                "env": {
                    "type": "object",
                    "additionalProperties": { "type": "string" },
                    "description": "Optional environment overrides."
                },
                "allow_net": { "type": "boolean", "default": false }
            },
            "anyOf": [
                { "required": ["command"] },
                { "required": ["program"] }
            ]
        })
    }
    async fn call(&self, i: Value) -> Result<Value, ToolError> {
        self.call_with_events(i, &mut |_| {}).await
    }

    async fn call_with_events(
        &self,
        i: Value,
        report: &mut (dyn FnMut(ToolProgressEvent) + Send),
    ) -> Result<Value, ToolError> {
        let request = BashCommandInput::from_value(i)?;
        let cwd = request.working_dir()?;
        let command_env = sandbox_command_env(&request.env);
        let wrapped = if let Some(command) = request.command.as_deref() {
            self.sandbox
                .wrap_shell_command(command, &cwd, request.allow_net)
                .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?
        } else {
            let program = request
                .program
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| ToolError::InvalidInput("program".into()))?;
            self.sandbox
                .wrap_exec_command(program, &request.args, &cwd, request.allow_net)
                .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?
        };

        ensure_sandbox_home_dirs().await?;

        let mut command = Command::new(&wrapped.program);
        command
            .args(&wrapped.args)
            .current_dir(&cwd)
            .envs(&command_env)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn()?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ToolError::ExecutionFailed("stdout pipe unavailable".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| ToolError::ExecutionFailed("stderr pipe unavailable".into()))?;

        let (tx, mut rx) = mpsc::unbounded_channel();
        let stdout_task = tokio::spawn(read_stream_chunks(
            stdout,
            BashStreamKind::Stdout,
            tx.clone(),
        ));
        let stderr_task = tokio::spawn(read_stream_chunks(stderr, BashStreamKind::Stderr, tx));

        let mut stdout_text = String::new();
        let mut stderr_text = String::new();
        let mut live_streamed = false;
        while let Some((stream, chunk)) = rx.recv().await {
            if chunk.is_empty() {
                continue;
            }
            live_streamed = true;
            match stream {
                BashStreamKind::Stdout => stdout_text.push_str(&chunk),
                BashStreamKind::Stderr => stderr_text.push_str(&chunk),
            }
            report(ToolProgressEvent::Output {
                stream: stream.output_stream(),
                chunk,
            });
        }

        let status = child.wait().await?;
        stdout_task
            .await
            .map_err(|err| ToolError::ExecutionFailed(err.to_string()))??;
        stderr_task
            .await
            .map_err(|err| ToolError::ExecutionFailed(err.to_string()))??;
        if let Some(path) = wrapped.cleanup_path.as_ref() {
            let _ = fs::remove_file(path).await;
        }
        Ok(json!({
            "stdout": stdout_text,
            "stderr": stderr_text,
            "exit_code": status.code(),
            "live_streamed": live_streamed,
        }))
    }
}

async fn read_stream_chunks<R>(
    reader: R,
    stream: BashStreamKind,
    tx: mpsc::UnboundedSender<(BashStreamKind, String)>,
) -> Result<(), ToolError>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    let mut reader = reader;
    let mut buffer = [0_u8; 4096];
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        let chunk = String::from_utf8_lossy(&buffer[..read]).into_owned();
        let _ = tx.send((stream, chunk));
    }
    Ok(())
}

async fn ensure_sandbox_home_dirs() -> Result<(), ToolError> {
    let config_dir = format!("{SANDBOX_HOME}/.config");
    let cache_dir = format!("{SANDBOX_HOME}/.cache");
    let local_dir = format!("{SANDBOX_HOME}/.local");
    let state_dir = format!("{SANDBOX_HOME}/.local/state");
    let share_dir = format!("{SANDBOX_HOME}/.local/share");

    for dir in [
        SANDBOX_HOME,
        config_dir.as_str(),
        cache_dir.as_str(),
        local_dir.as_str(),
        state_dir.as_str(),
        share_dir.as_str(),
    ] {
        fs::create_dir_all(dir).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{sandbox_command_env, BashCommandInput, BashTool, SANDBOX_HOME};
    use crate::sandbox::SandboxManager;
    use crate::tool::{Tool, ToolOutputStream, ToolProgressEvent};
    use serde_json::{json, Value};
    use std::collections::HashMap;
    use std::env;
    use std::path::Path;
    use std::sync::Arc;
    use tempfile::tempdir;

    #[test]
    fn parses_legacy_shell_payload() {
        let input = BashCommandInput::from_value(json!({
            "command": "cargo test",
            "allow_net": true
        }))
        .expect("legacy payload");

        assert_eq!(input.command.as_deref(), Some("cargo test"));
        assert!(input.allow_net);
        assert_eq!(input.summary(), "cargo test");
    }

    #[test]
    fn parses_structured_payload() {
        let input = BashCommandInput::from_value(json!({
            "program": "cargo",
            "args": ["check", "--workspace"],
            "cwd": "/tmp/workspace",
            "env": { "RUST_LOG": "debug" },
            "allow_net": false
        }))
        .expect("structured payload");

        assert_eq!(input.program.as_deref(), Some("cargo"));
        assert_eq!(
            input.args,
            vec!["check".to_string(), "--workspace".to_string()]
        );
        assert_eq!(input.cwd.as_deref(), Some("/tmp/workspace"));
        assert_eq!(input.env.get("RUST_LOG").map(String::as_str), Some("debug"));
        assert_eq!(input.summary(), "cargo check --workspace");
    }

    #[test]
    fn sandbox_command_env_defaults_home_and_xdg_roots() {
        let env_map = sandbox_command_env(&HashMap::new());

        assert_eq!(env_map.get("HOME").map(String::as_str), Some(SANDBOX_HOME));
        assert_eq!(
            env_map.get("XDG_CONFIG_HOME").map(String::as_str),
            Some("/tmp/rara-home/.config")
        );
        assert_eq!(
            env_map.get("XDG_CACHE_HOME").map(String::as_str),
            Some("/tmp/rara-home/.cache")
        );
    }

    #[test]
    fn sandbox_command_env_keeps_explicit_overrides() {
        let env_map = sandbox_command_env(&HashMap::from([
            ("HOME".to_string(), "/custom/home".to_string()),
            (
                "XDG_CACHE_HOME".to_string(),
                "/custom/home/.cache".to_string(),
            ),
        ]));

        assert_eq!(
            env_map.get("HOME").map(String::as_str),
            Some("/custom/home")
        );
        assert_eq!(
            env_map.get("XDG_CACHE_HOME").map(String::as_str),
            Some("/custom/home/.cache")
        );
        assert_eq!(
            env_map.get("XDG_CONFIG_HOME").map(String::as_str),
            Some("/tmp/rara-home/.config")
        );
    }

    #[tokio::test]
    async fn streaming_call_reports_stdout_and_stderr_chunks() {
        let temp = tempdir().expect("tempdir");
        let sandbox = SandboxManager::new_for_rara_dir(temp.path().join(".rara")).expect("sandbox");
        let wrapped = sandbox
            .wrap_exec_command(
                "sh",
                &[
                    "-c".to_string(),
                    "printf 'out\\n'; printf 'err\\n' >&2".to_string(),
                ],
                temp.path().to_string_lossy().as_ref(),
                false,
            )
            .expect("wrapped command");
        if !binary_exists(&wrapped.program) {
            return;
        }
        let tool = BashTool {
            sandbox: Arc::new(sandbox),
        };
        let mut events = Vec::new();
        let result = tool
            .call_with_events(
                json!({
                    "program": "sh",
                    "args": ["-c", "printf 'out\\n'; printf 'err\\n' >&2"],
                }),
                &mut |event| events.push(event),
            )
            .await
            .expect("bash result");

        assert!(!events.is_empty());
        assert!(events.iter().any(|event| matches!(
            event,
            ToolProgressEvent::Output {
                stream: ToolOutputStream::Stdout | ToolOutputStream::Stderr,
                ..
            }
        )));
        assert_eq!(
            result.get("live_streamed").and_then(Value::as_bool),
            Some(true)
        );
    }

    fn binary_exists(program: &str) -> bool {
        let program_path = Path::new(program);
        if program_path.components().count() > 1 {
            return program_path.exists();
        }

        env::var_os("PATH")
            .map(|paths| env::split_paths(&paths).any(|dir| dir.join(program).exists()))
            .unwrap_or(false)
    }
}
