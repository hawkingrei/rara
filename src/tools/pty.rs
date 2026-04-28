use crate::sandbox::{sandbox_failure_hint, SandboxManager, WrappedCommand};
use crate::tool::{Tool, ToolError};
use async_trait::async_trait;
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::env;
use std::fs::OpenOptions;
use std::io::{Read, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use uuid::Uuid;

pub struct PtyStartTool {
    pub sessions: Arc<PtySessionStore>,
    pub sandbox: Arc<SandboxManager>,
}

pub struct PtyReadTool {
    pub sessions: Arc<PtySessionStore>,
}

pub struct PtyWriteTool {
    pub sessions: Arc<PtySessionStore>,
}

pub struct PtyKillTool {
    pub sessions: Arc<PtySessionStore>,
}

pub struct PtySessionStore {
    dir: PathBuf,
    sessions: Mutex<HashMap<String, PtySessionRecord>>,
}

struct PtySessionRecord {
    id: String,
    command: String,
    output_path: PathBuf,
    sandboxed: bool,
    sandbox_backend: String,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    child: Arc<Mutex<Box<dyn portable_pty::Child + Send + Sync>>>,
    status: Arc<Mutex<PtySessionStatus>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PtySessionStatus {
    Running,
    Completed,
    Killed,
}

impl PtySessionStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Killed => "killed",
        }
    }
}

impl PtySessionStore {
    pub fn new(dir: PathBuf) -> Result<Self, ToolError> {
        std::fs::create_dir_all(&dir)?;
        Ok(Self {
            dir,
            sessions: Mutex::new(HashMap::new()),
        })
    }

    fn start(
        &self,
        command: String,
        wrapped: WrappedCommand,
        cwd: String,
        env: HashMap<String, String>,
        rows: u16,
        cols: u16,
    ) -> Result<PtySessionSnapshot, ToolError> {
        let id = format!("pty-{}", Uuid::new_v4());
        let output_path = self.dir.join(format!("{id}.log"));
        let pty_system = NativePtySystem::default();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|err| ToolError::ExecutionFailed(format!("open pty: {err}")))?;
        let command_env = command_env_for_wrapped(&wrapped, &env)?;
        if wrapped.sandboxed && wrapped.sandbox_backend == "macos-seatbelt" {
            let sandbox_home = wrapped.sandbox_home.as_deref().ok_or_else(|| {
                ToolError::ExecutionFailed("sandboxed pty is missing sandbox home".into())
            })?;
            ensure_sandbox_home_dirs(sandbox_home)?;
        }

        let mut cmd = CommandBuilder::new(&wrapped.program);
        for arg in &wrapped.args {
            cmd.arg(arg);
        }
        cmd.cwd(&cwd);
        if wrapped.sandboxed {
            cmd.env_clear();
        }
        for (key, value) in command_env {
            cmd.env(key, value);
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|err| ToolError::ExecutionFailed(format!("spawn pty command: {err}")))?;
        drop(pair.slave);
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|err| ToolError::ExecutionFailed(format!("clone pty reader: {err}")))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|err| ToolError::ExecutionFailed(format!("take pty writer: {err}")))?;
        let child = Arc::new(Mutex::new(child));
        let status = Arc::new(Mutex::new(PtySessionStatus::Running));
        let reader_status = status.clone();
        let reader_path = output_path.clone();

        thread::spawn(move || {
            let mut file = match OpenOptions::new()
                .create(true)
                .append(true)
                .open(&reader_path)
            {
                Ok(file) => file,
                Err(_) => return,
            };
            let mut buffer = [0_u8; 4096];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        let _ = file.write_all(&buffer[..n]);
                        let _ = file.flush();
                    }
                    Err(_) => break,
                }
            }
            let mut status = reader_status.lock().expect("pty status lock");
            if matches!(*status, PtySessionStatus::Running) {
                *status = PtySessionStatus::Completed;
            }
        });

        let record = PtySessionRecord {
            id: id.clone(),
            command,
            output_path,
            sandboxed: wrapped.sandboxed,
            sandbox_backend: wrapped.sandbox_backend,
            writer: Arc::new(Mutex::new(writer)),
            child,
            status,
        };
        let snapshot = record.snapshot();
        self.sessions
            .lock()
            .expect("pty session store lock")
            .insert(id, record);
        Ok(snapshot)
    }

    fn get(&self, id: &str) -> Option<PtySessionSnapshot> {
        self.sessions
            .lock()
            .expect("pty session store lock")
            .get(id)
            .map(PtySessionRecord::snapshot)
    }

    fn write(&self, id: &str, input: &str) -> Result<PtySessionSnapshot, ToolError> {
        let writer = {
            let sessions = self.sessions.lock().expect("pty session store lock");
            let record = sessions
                .get(id)
                .ok_or_else(|| ToolError::InvalidInput(format!("unknown pty session id: {id}")))?;
            record.writer.clone()
        };
        let mut writer = writer.lock().expect("pty writer lock");
        writer.write_all(input.as_bytes())?;
        writer.flush()?;
        drop(writer);
        self.get(id)
            .ok_or_else(|| ToolError::InvalidInput(format!("unknown pty session id: {id}")))
    }

    fn kill(&self, id: &str) -> Result<PtySessionSnapshot, ToolError> {
        let (child, status, mut snapshot) = {
            let sessions = self.sessions.lock().expect("pty session store lock");
            let record = sessions
                .get(id)
                .ok_or_else(|| ToolError::InvalidInput(format!("unknown pty session id: {id}")))?;
            (
                record.child.clone(),
                record.status.clone(),
                record.snapshot(),
            )
        };
        child
            .lock()
            .expect("pty child lock")
            .kill()
            .map_err(|err| ToolError::ExecutionFailed(format!("kill pty session: {err}")))?;
        *status.lock().expect("pty status lock") = PtySessionStatus::Killed;
        snapshot.status = PtySessionStatus::Killed;
        Ok(snapshot)
    }
}

struct PtySessionSnapshot {
    id: String,
    command: String,
    output_path: PathBuf,
    sandboxed: bool,
    sandbox_backend: String,
    status: PtySessionStatus,
}

impl PtySessionRecord {
    fn snapshot(&self) -> PtySessionSnapshot {
        PtySessionSnapshot {
            id: self.id.clone(),
            command: self.command.clone(),
            output_path: self.output_path.clone(),
            sandboxed: self.sandboxed,
            sandbox_backend: self.sandbox_backend.clone(),
            status: *self.status.lock().expect("pty status lock"),
        }
    }
}

impl PtySessionSnapshot {
    async fn into_json(self, tail_bytes: usize) -> Result<Value, ToolError> {
        let output = read_output_tail(&self.output_path, tail_bytes).await?;
        Ok(json!({
            "session_id": self.id,
            "command": self.command,
            "status": self.status.as_str(),
            "output_path": self.output_path,
            "sandboxed": self.sandboxed,
            "sandbox_backend": self.sandbox_backend,
            "output": output,
        }))
    }
}

#[async_trait]
impl Tool for PtyStartTool {
    fn name(&self) -> &str {
        "pty_start"
    }

    fn description(&self) -> &str {
        "Start an interactive PTY session for commands that need terminal input"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "Shell command to run inside a PTY." },
                "cwd": { "type": "string", "description": "Optional working directory." },
                "env": {
                    "type": "object",
                    "additionalProperties": { "type": "string" },
                    "description": "Optional environment overrides."
                },
                "allow_net": { "type": "boolean", "default": false },
                "rows": { "type": "integer", "default": 24, "minimum": 1 },
                "cols": { "type": "integer", "default": 120, "minimum": 1 }
            },
            "required": ["command"]
        })
    }

    async fn call(&self, input: Value) -> Result<Value, ToolError> {
        let command = input["command"]
            .as_str()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| ToolError::InvalidInput("command".into()))?
            .to_string();
        let cwd = input
            .get("cwd")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                env::current_dir()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|_| ".".to_string())
            });
        let env = parse_env(input.get("env"))?;
        let allow_net = input
            .get("allow_net")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let wrapped = self
            .sandbox
            .wrap_pty_shell_command(&command, &cwd, allow_net)
            .map_err(|err| {
                ToolError::ExecutionFailed(format!("{} {}", err, sandbox_failure_hint()))
            })?;
        let rows = input.get("rows").and_then(Value::as_u64).unwrap_or(24) as u16;
        let cols = input.get("cols").and_then(Value::as_u64).unwrap_or(120) as u16;
        self.sessions
            .start(command, wrapped, cwd, env, rows.max(1), cols.max(1))?
            .into_json(12_000)
            .await
    }
}

#[async_trait]
impl Tool for PtyReadTool {
    fn name(&self) -> &str {
        "pty_read"
    }

    fn description(&self) -> &str {
        "Read recent output from a PTY session"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string" },
                "tail_bytes": { "type": "integer", "default": 12000, "minimum": 1 }
            },
            "required": ["session_id"]
        })
    }

    async fn call(&self, input: Value) -> Result<Value, ToolError> {
        let session_id = input["session_id"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidInput("session_id".into()))?;
        let tail_bytes = input
            .get("tail_bytes")
            .and_then(Value::as_u64)
            .unwrap_or(12_000)
            .min(1_000_000) as usize;
        self.sessions
            .get(session_id)
            .ok_or_else(|| {
                ToolError::InvalidInput(format!("unknown pty session id: {session_id}"))
            })?
            .into_json(tail_bytes)
            .await
    }
}

#[async_trait]
impl Tool for PtyWriteTool {
    fn name(&self) -> &str {
        "pty_write"
    }

    fn description(&self) -> &str {
        "Write input to a running PTY session"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string" },
                "input": { "type": "string", "description": "Text to write, including newlines or control characters when needed." }
            },
            "required": ["session_id", "input"]
        })
    }

    async fn call(&self, input: Value) -> Result<Value, ToolError> {
        let session_id = input["session_id"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidInput("session_id".into()))?;
        let text = input["input"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidInput("input".into()))?;
        self.sessions
            .write(session_id, text)?
            .into_json(12_000)
            .await
    }
}

#[async_trait]
impl Tool for PtyKillTool {
    fn name(&self) -> &str {
        "pty_kill"
    }

    fn description(&self) -> &str {
        "Kill a PTY session"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string" }
            },
            "required": ["session_id"]
        })
    }

    async fn call(&self, input: Value) -> Result<Value, ToolError> {
        let session_id = input["session_id"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidInput("session_id".into()))?;
        self.sessions.kill(session_id)?.into_json(12_000).await
    }
}

fn parse_env(value: Option<&Value>) -> Result<HashMap<String, String>, ToolError> {
    let Some(value) = value else {
        return Ok(HashMap::new());
    };
    serde_json::from_value(value.clone())
        .map_err(|err| ToolError::InvalidInput(format!("env: {err}")))
}

fn command_env_for_wrapped(
    wrapped: &WrappedCommand,
    overrides: &HashMap<String, String>,
) -> Result<HashMap<String, String>, ToolError> {
    let mut env_map = HashMap::new();
    if wrapped.sandboxed {
        let sandbox_home = wrapped.sandbox_home.as_deref().ok_or_else(|| {
            ToolError::ExecutionFailed("sandboxed pty is missing sandbox home".into())
        })?;
        env_map.insert("HOME".to_string(), sandbox_home.display().to_string());
        env_map.insert(
            "XDG_CACHE_HOME".to_string(),
            sandbox_home.join(".cache").display().to_string(),
        );
        env_map.insert(
            "XDG_CONFIG_HOME".to_string(),
            sandbox_home.join(".config").display().to_string(),
        );
        env_map.insert(
            "XDG_DATA_HOME".to_string(),
            sandbox_home.join(".local/share").display().to_string(),
        );
    }
    if let Ok(path) = env::var("PATH") {
        env_map.insert("PATH".to_string(), path);
    }
    env_map.extend(overrides.clone());
    Ok(env_map)
}

fn ensure_sandbox_home_dirs(sandbox_home: &Path) -> Result<(), ToolError> {
    for dir in [
        sandbox_home,
        &sandbox_home.join(".cache"),
        &sandbox_home.join(".config"),
        &sandbox_home.join(".local"),
        &sandbox_home.join(".local/share"),
    ] {
        std::fs::create_dir_all(dir)?;
    }
    Ok(())
}

async fn read_output_tail(path: &Path, max_bytes: usize) -> Result<String, ToolError> {
    let mut file = match tokio::fs::File::open(path).await {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
        Err(err) => return Err(err.into()),
    };
    let file_len = file.metadata().await?.len();
    let start = file_len.saturating_sub(max_bytes as u64);
    file.seek(SeekFrom::Start(start)).await?;
    let mut bytes = Vec::with_capacity(max_bytes.min(file_len as usize));
    file.read_to_end(&mut bytes).await?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::WrappedCommand;
    use tempfile::tempdir;

    #[tokio::test]
    async fn read_output_tail_returns_only_requested_suffix() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("pty.log");
        tokio::fs::write(&path, b"0123456789tail")
            .await
            .expect("write log");

        let output = read_output_tail(&path, 4).await.expect("tail");

        assert_eq!(output, "tail");
    }

    #[tokio::test]
    async fn pty_session_accepts_input_and_exposes_output() {
        let temp = tempdir().expect("tempdir");
        let sessions = Arc::new(PtySessionStore::new(temp.path().join("pty")).expect("pty store"));
        let write = PtyWriteTool {
            sessions: sessions.clone(),
        };
        let read = PtyReadTool {
            sessions: sessions.clone(),
        };

        let command = "read line; printf \"got:%s\\n\" \"$line\"".to_string();
        let started = sessions
            .start(
                command.clone(),
                WrappedCommand {
                    program: "/bin/sh".to_string(),
                    args: vec!["-c".to_string(), command],
                    cleanup_path: None,
                    sandboxed: false,
                    sandbox_backend: "direct".to_string(),
                    sandbox_home: None,
                },
                temp.path().display().to_string(),
                HashMap::new(),
                24,
                120,
            )
            .expect("start pty")
            .into_json(12_000)
            .await
            .expect("pty json");
        let session_id = started
            .get("session_id")
            .and_then(Value::as_str)
            .expect("session id")
            .to_string();

        write
            .call(json!({
                "session_id": session_id,
                "input": "hello from pty\n",
            }))
            .await
            .expect("write pty");

        let mut last = Value::Null;
        for _ in 0..50 {
            last = read
                .call(json!({ "session_id": session_id, "tail_bytes": 4096 }))
                .await
                .expect("read pty");
            if last
                .get("output")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .contains("got:hello from pty")
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        let output = last
            .get("output")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(
            output.contains("got:hello from pty"),
            "last pty output did not contain expected marker: {last}"
        );
    }
}
