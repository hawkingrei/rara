use crate::sandbox::{sandbox_failure_hint, SandboxManager, WrappedCommand};
use crate::tool::{Tool, ToolError, ToolOutputStream, ToolProgressEvent};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::env;
use std::io::SeekFrom;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex;
use tokio::fs;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use uuid::Uuid;

pub struct BashTool {
    pub sandbox: Arc<SandboxManager>,
    pub background_tasks: Arc<BackgroundTaskStore>,
}

pub struct BackgroundTaskStatusTool {
    pub background_tasks: Arc<BackgroundTaskStore>,
}

pub struct BackgroundTaskListTool {
    pub background_tasks: Arc<BackgroundTaskStore>,
}

pub struct BackgroundTaskStopTool {
    pub background_tasks: Arc<BackgroundTaskStore>,
}

#[derive(Debug, Clone)]
pub struct BackgroundTaskStore {
    dir: PathBuf,
    tasks: Arc<Mutex<HashMap<String, BackgroundTaskRecord>>>,
    stop_signals: Arc<Mutex<HashMap<String, oneshot::Sender<()>>>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BackgroundTaskRecord {
    id: String,
    command: String,
    output_path: PathBuf,
    status: BackgroundTaskStatus,
    exit_code: Option<i32>,
    sandboxed: bool,
    sandbox_backend: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundTaskStatus {
    Running,
    Completed,
    Failed,
    Killed,
}

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
    #[serde(default)]
    pub run_in_background: bool,
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

impl BackgroundTaskStore {
    pub fn new(dir: PathBuf) -> Result<Self, ToolError> {
        std::fs::create_dir_all(&dir)?;
        Ok(Self {
            dir,
            tasks: Arc::new(Mutex::new(HashMap::new())),
            stop_signals: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    fn start_record(
        &self,
        command: String,
        sandboxed: bool,
        sandbox_backend: String,
    ) -> Result<(BackgroundTaskRecord, oneshot::Receiver<()>), ToolError> {
        let id = format!("bash-{}", Uuid::new_v4());
        let output_path = self.dir.join(format!("{id}.log"));
        let record = BackgroundTaskRecord {
            id: id.clone(),
            command,
            output_path,
            status: BackgroundTaskStatus::Running,
            exit_code: None,
            sandboxed,
            sandbox_backend,
        };
        let (stop_tx, stop_rx) = oneshot::channel();
        self.tasks
            .lock()
            .expect("background task store lock")
            .insert(id.clone(), record.clone());
        self.stop_signals
            .lock()
            .expect("background task stop signal lock")
            .insert(id, stop_tx);
        Ok((record, stop_rx))
    }

    fn finish(&self, id: &str, status: BackgroundTaskStatus, exit_code: Option<i32>) {
        if let Some(record) = self
            .tasks
            .lock()
            .expect("background task store lock")
            .get_mut(id)
        {
            if !matches!(record.status, BackgroundTaskStatus::Killed) {
                record.status = status;
            }
            record.exit_code = exit_code;
        }
        self.stop_signals
            .lock()
            .expect("background task stop signal lock")
            .remove(id);
    }

    fn get(&self, id: &str) -> Option<BackgroundTaskRecord> {
        self.tasks
            .lock()
            .expect("background task store lock")
            .get(id)
            .cloned()
    }

    fn list(&self) -> Vec<BackgroundTaskRecord> {
        let mut records = self
            .tasks
            .lock()
            .expect("background task store lock")
            .values()
            .cloned()
            .collect::<Vec<_>>();
        records.sort_by(|left, right| left.id.cmp(&right.id));
        records
    }

    fn stop(&self, id: &str) -> Result<BackgroundTaskRecord, ToolError> {
        let mut tasks = self.tasks.lock().expect("background task store lock");
        let record = tasks
            .get_mut(id)
            .ok_or_else(|| ToolError::InvalidInput(format!("unknown task id: {id}")))?;
        if !matches!(record.status, BackgroundTaskStatus::Running) {
            return Ok(record.clone());
        }
        record.status = BackgroundTaskStatus::Killed;
        let stopped = record.clone();
        drop(tasks);

        if let Some(stop) = self
            .stop_signals
            .lock()
            .expect("background task stop signal lock")
            .remove(id)
        {
            let _ = stop.send(());
        }
        Ok(stopped)
    }

    fn stop_all(&self) -> Vec<BackgroundTaskRecord> {
        let ids = self
            .list()
            .into_iter()
            .filter(|record| matches!(record.status, BackgroundTaskStatus::Running))
            .map(|record| record.id)
            .collect::<Vec<_>>();
        ids.into_iter()
            .filter_map(|id| self.stop(&id).ok())
            .collect()
    }
}

fn sandbox_command_env(
    sandbox_home: &Path,
    overrides: &HashMap<String, String>,
) -> HashMap<String, String> {
    let sandbox_home = sandbox_home.to_string_lossy();
    let mut env_map = HashMap::from([
        ("HOME".to_string(), sandbox_home.to_string()),
        (
            "XDG_CONFIG_HOME".to_string(),
            format!("{sandbox_home}/.config"),
        ),
        (
            "XDG_CACHE_HOME".to_string(),
            format!("{sandbox_home}/.cache"),
        ),
        (
            "XDG_STATE_HOME".to_string(),
            format!("{sandbox_home}/.local/state"),
        ),
        (
            "XDG_DATA_HOME".to_string(),
            format!("{sandbox_home}/.local/share"),
        ),
    ]);
    if let Ok(path) = env::var("PATH") {
        env_map.insert("PATH".to_string(), path);
    }
    env_map.extend(overrides.clone());
    env_map
}

fn command_env_for_wrapped(
    wrapped: &WrappedCommand,
    overrides: &HashMap<String, String>,
) -> Result<HashMap<String, String>, ToolError> {
    if wrapped.sandboxed {
        let sandbox_home = wrapped.sandbox_home.as_deref().ok_or_else(|| {
            ToolError::ExecutionFailed("sandboxed command is missing sandbox home".into())
        })?;
        Ok(sandbox_command_env(sandbox_home, overrides))
    } else {
        Ok(overrides.clone())
    }
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
                "allow_net": { "type": "boolean", "default": false },
                "run_in_background": {
                    "type": "boolean",
                    "default": false,
                    "description": "Run the command as a background task and return a task id immediately. Use background_task_status to inspect output later."
                }
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
        let wrapped = if let Some(command) = request.command.as_deref() {
            self.sandbox
                .wrap_shell_command(command, &cwd, request.allow_net)
                .map_err(|e| {
                    ToolError::ExecutionFailed(format!("{} {}", e, sandbox_failure_hint()))
                })?
        } else {
            let program = request
                .program
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| ToolError::InvalidInput("program".into()))?;
            self.sandbox
                .wrap_exec_command(program, &request.args, &cwd, request.allow_net)
                .map_err(|e| {
                    ToolError::ExecutionFailed(format!("{} {}", e, sandbox_failure_hint()))
                })?
        };
        let command_env = command_env_for_wrapped(&wrapped, &request.env)?;

        if wrapped.sandboxed && wrapped.sandbox_backend == "macos-seatbelt" {
            let sandbox_home = wrapped.sandbox_home.as_deref().ok_or_else(|| {
                ToolError::ExecutionFailed("sandboxed command is missing sandbox home".into())
            })?;
            ensure_sandbox_home_dirs(sandbox_home).await?;
        }

        let mut command = Command::new(&wrapped.program);
        if wrapped.sandboxed {
            command.env_clear();
        }
        command
            .args(&wrapped.args)
            .current_dir(&cwd)
            .envs(&command_env)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().map_err(|err| {
            if wrapped.sandboxed {
                ToolError::ExecutionFailed(format!(
                    "failed to launch sandbox '{}': {err}. {}",
                    wrapped.program,
                    sandbox_failure_hint()
                ))
            } else {
                ToolError::ExecutionFailed(format!(
                    "failed to launch command '{}': {err}",
                    wrapped.program
                ))
            }
        })?;

        if request.run_in_background {
            let (record, stop_rx) = self.background_tasks.start_record(
                request.summary(),
                wrapped.sandboxed,
                wrapped.sandbox_backend.clone(),
            )?;
            spawn_background_bash_task(
                child,
                wrapped,
                record.clone(),
                self.background_tasks.clone(),
                stop_rx,
            );
            return Ok(json!({
                "stdout": "",
                "stderr": "",
                "exit_code": null,
                "live_streamed": false,
                "sandboxed": record.sandboxed,
                "sandbox_backend": record.sandbox_backend,
                "background_task_id": record.id,
                "output_path": record.output_path,
                "status": record.status,
            }));
        }

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ToolError::ExecutionFailed("stdout pipe unavailable".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| ToolError::ExecutionFailed("stderr pipe unavailable".into()))?;

        let (tx, mut rx) = mpsc::channel(64);
        let stdout_task = tokio::spawn(read_stream_chunks(
            stdout,
            BashStreamKind::Stdout,
            tx.clone(),
        ));
        let stderr_task = tokio::spawn(read_stream_chunks(stderr, BashStreamKind::Stderr, tx));

        let mut stdout_text = String::new();
        let mut stderr_text = String::new();
        let mut live_streamed = false;
        if !wrapped.sandboxed {
            let chunk = unsandboxed_execution_warning(&wrapped);
            stderr_text.push_str(&chunk);
            live_streamed = true;
            report(ToolProgressEvent::Output {
                stream: ToolOutputStream::Stderr,
                chunk,
            });
        }
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
        if wrapped.sandboxed {
            if let Some(hint) = sandbox_output_hint(&stderr_text) {
                stderr_text.push_str(hint);
            }
        }

        Ok(json!({
            "stdout": stdout_text,
            "stderr": stderr_text,
            "exit_code": status.code(),
            "live_streamed": live_streamed,
            "sandboxed": wrapped.sandboxed,
            "sandbox_backend": wrapped.sandbox_backend,
        }))
    }
}

#[async_trait]
impl Tool for BackgroundTaskListTool {
    fn name(&self) -> &str {
        "background_task_list"
    }

    fn description(&self) -> &str {
        "List background bash tasks"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn call(&self, _input: Value) -> Result<Value, ToolError> {
        Ok(json!({
            "tasks": self.background_tasks.list(),
        }))
    }
}

#[async_trait]
impl Tool for BackgroundTaskStatusTool {
    fn name(&self) -> &str {
        "background_task_status"
    }

    fn description(&self) -> &str {
        "Inspect a background bash task and read the tail of its output"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "Background task id returned by bash run_in_background."
                },
                "tail_bytes": {
                    "type": "integer",
                    "minimum": 1,
                    "default": 12000,
                    "description": "Maximum number of output bytes to return from the end of the task log."
                }
            },
            "required": ["task_id"]
        })
    }

    async fn call(&self, input: Value) -> Result<Value, ToolError> {
        let task_id = input["task_id"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidInput("task_id".into()))?;
        let tail_bytes = input
            .get("tail_bytes")
            .and_then(Value::as_u64)
            .unwrap_or(12_000)
            .min(1_000_000) as usize;
        let record = self
            .background_tasks
            .get(task_id)
            .ok_or_else(|| ToolError::InvalidInput(format!("unknown task id: {task_id}")))?;
        let output = read_output_tail(&record.output_path, tail_bytes).await?;

        Ok(json!({
            "task_id": record.id,
            "command": record.command,
            "status": record.status,
            "exit_code": record.exit_code,
            "output_path": record.output_path,
            "output": output,
            "sandboxed": record.sandboxed,
            "sandbox_backend": record.sandbox_backend,
        }))
    }
}

#[async_trait]
impl Tool for BackgroundTaskStopTool {
    fn name(&self) -> &str {
        "background_task_stop"
    }

    fn description(&self) -> &str {
        "Stop one background bash task, or all running background bash tasks when task_id is omitted"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "Background task id returned by bash run_in_background. Omit to stop all running background bash tasks."
                }
            }
        })
    }

    async fn call(&self, input: Value) -> Result<Value, ToolError> {
        if let Some(task_id) = input.get("task_id").and_then(Value::as_str) {
            let task = self.background_tasks.stop(task_id)?;
            return Ok(json!({ "stopped": [task] }));
        }
        Ok(json!({ "stopped": self.background_tasks.stop_all() }))
    }
}

fn spawn_background_bash_task(
    mut child: Child,
    wrapped: WrappedCommand,
    record: BackgroundTaskRecord,
    store: Arc<BackgroundTaskStore>,
    stop_rx: oneshot::Receiver<()>,
) {
    tokio::spawn(async move {
        let result = run_background_bash_task(&mut child, wrapped, &record, stop_rx).await;
        let (status, exit_code) = match result {
            Ok(code) => {
                if code == Some(0) {
                    (BackgroundTaskStatus::Completed, code)
                } else {
                    (BackgroundTaskStatus::Failed, code)
                }
            }
            Err(err) => {
                let _ = append_background_output(
                    &record.output_path,
                    BashStreamKind::Stderr,
                    &format!("background task failed: {err}\n"),
                )
                .await;
                (BackgroundTaskStatus::Failed, None)
            }
        };
        store.finish(&record.id, status, exit_code);
    });
}

async fn run_background_bash_task(
    child: &mut Child,
    wrapped: WrappedCommand,
    record: &BackgroundTaskRecord,
    mut stop_rx: oneshot::Receiver<()>,
) -> Result<Option<i32>, ToolError> {
    if let Some(parent) = record.output_path.parent() {
        fs::create_dir_all(parent).await?;
    }
    fs::write(&record.output_path, "").await?;
    if !wrapped.sandboxed {
        append_background_output(
            &record.output_path,
            BashStreamKind::Stderr,
            &unsandboxed_execution_warning(&wrapped),
        )
        .await?;
    }

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ToolError::ExecutionFailed("stdout pipe unavailable".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| ToolError::ExecutionFailed("stderr pipe unavailable".into()))?;
    let (tx, mut rx) = mpsc::channel(64);
    let stdout_task = tokio::spawn(read_stream_chunks(
        stdout,
        BashStreamKind::Stdout,
        tx.clone(),
    ));
    let stderr_task = tokio::spawn(read_stream_chunks(stderr, BashStreamKind::Stderr, tx));

    let mut output_file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&record.output_path)
        .await?;
    let mut stop_requested = false;
    loop {
        tokio::select! {
            chunk = rx.recv() => {
                let Some((stream, chunk)) = chunk else {
                    break;
                };
                if !chunk.is_empty() {
                    match stream {
                        BashStreamKind::Stdout => output_file.write_all(chunk.as_bytes()).await?,
                        BashStreamKind::Stderr => {
                            output_file.write_all(b"[stderr] ").await?;
                            output_file.write_all(chunk.as_bytes()).await?;
                        }
                    }
                }
            }
            _ = &mut stop_rx, if !stop_requested => {
                stop_requested = true;
                child.start_kill()
                    .map_err(|err| ToolError::ExecutionFailed(format!("stop background task: {err}")))?;
                output_file.write_all(b"[stderr] background task stop requested\n").await?;
            }
        }
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
    Ok(status.code())
}

async fn append_background_output(
    path: &Path,
    stream: BashStreamKind,
    chunk: &str,
) -> Result<(), ToolError> {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await?;
    match stream {
        BashStreamKind::Stdout => file.write_all(chunk.as_bytes()).await?,
        BashStreamKind::Stderr => {
            file.write_all(b"[stderr] ").await?;
            file.write_all(chunk.as_bytes()).await?;
        }
    }
    Ok(())
}

async fn read_output_tail(path: &Path, max_bytes: usize) -> Result<String, ToolError> {
    let mut file = match fs::File::open(path).await {
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

async fn read_stream_chunks<R>(
    reader: R,
    stream: BashStreamKind,
    tx: mpsc::Sender<(BashStreamKind, String)>,
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
        if tx.send((stream, chunk)).await.is_err() {
            break;
        }
    }
    Ok(())
}

fn sandbox_output_hint(stderr: &str) -> Option<&'static str> {
    let lower = stderr.to_ascii_lowercase();
    if lower.contains("sandbox: violation")
        || lower.contains("operation not permitted")
        || lower.contains("command not found")
        || lower.contains("no such file or directory")
        || lower.contains("permission denied")
    {
        Some("\n\nhint: Sandboxed bash appears blocked or missing a runtime path. Prefer direct file tools such as read_file, apply_patch, and replace_lines; ask the user only if a real shell command is required.\n")
    } else {
        None
    }
}

fn unsandboxed_execution_warning(wrapped: &WrappedCommand) -> String {
    format!(
        "warning: command is running without sandbox isolation (backend: {}).\n",
        wrapped.sandbox_backend
    )
}

async fn ensure_sandbox_home_dirs(sandbox_home: &Path) -> Result<(), ToolError> {
    for dir in [
        sandbox_home.to_path_buf(),
        sandbox_home.join(".config"),
        sandbox_home.join(".cache"),
        sandbox_home.join(".local"),
        sandbox_home.join(".local/state"),
        sandbox_home.join(".local/share"),
    ] {
        fs::create_dir_all(dir).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        command_env_for_wrapped, read_output_tail, sandbox_command_env, sandbox_output_hint,
        unsandboxed_execution_warning, BackgroundTaskListTool, BackgroundTaskStatus,
        BackgroundTaskStatusTool, BackgroundTaskStopTool, BackgroundTaskStore, BashCommandInput,
        BashTool,
    };
    use crate::sandbox::{SandboxManager, WrappedCommand};
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
        assert!(!input.run_in_background);
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
        assert!(!input.run_in_background);
        assert_eq!(input.summary(), "cargo check --workspace");
    }

    #[test]
    fn parses_background_payload() {
        let input = BashCommandInput::from_value(json!({
            "program": "cargo",
            "args": ["test"],
            "run_in_background": true
        }))
        .expect("background payload");

        assert!(input.run_in_background);
        assert_eq!(input.summary(), "cargo test");
    }

    #[test]
    fn sandbox_command_env_defaults_home_and_xdg_roots() {
        let sandbox_home = Path::new("/tmp/rara-test-home");
        let original_path = std::env::var_os("PATH");
        std::env::set_var("PATH", "/custom/bin:/usr/bin");
        let env_map = sandbox_command_env(sandbox_home, &HashMap::new());

        assert_eq!(
            env_map.get("HOME").map(String::as_str),
            Some("/tmp/rara-test-home")
        );
        assert_eq!(
            env_map.get("XDG_CONFIG_HOME").map(String::as_str),
            Some("/tmp/rara-test-home/.config")
        );
        assert_eq!(
            env_map.get("XDG_CACHE_HOME").map(String::as_str),
            Some("/tmp/rara-test-home/.cache")
        );
        assert_eq!(
            env_map.get("PATH").map(String::as_str),
            Some("/custom/bin:/usr/bin")
        );
        if let Some(path) = original_path {
            std::env::set_var("PATH", path);
        } else {
            std::env::remove_var("PATH");
        }
    }

    #[test]
    fn sandbox_command_env_keeps_explicit_overrides() {
        let sandbox_home = Path::new("/tmp/rara-test-home");
        let env_map = sandbox_command_env(
            sandbox_home,
            &HashMap::from([
                ("HOME".to_string(), "/custom/home".to_string()),
                (
                    "XDG_CACHE_HOME".to_string(),
                    "/custom/home/.cache".to_string(),
                ),
                ("PATH".to_string(), "/override/bin".to_string()),
            ]),
        );

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
            Some("/tmp/rara-test-home/.config")
        );
        assert_eq!(
            env_map.get("PATH").map(String::as_str),
            Some("/override/bin")
        );
    }

    #[test]
    fn sandbox_output_hint_explains_blocked_shell_paths() {
        let hint = sandbox_output_hint("sandbox-exec: /bin/sed: Operation not permitted")
            .expect("sandbox hint");

        assert!(hint.contains("Prefer direct file tools"));
        assert!(hint.contains("replace_lines"));
    }

    #[test]
    fn direct_wrapped_command_keeps_caller_environment_overrides_only() {
        let wrapped = WrappedCommand {
            program: "sh".to_string(),
            args: vec!["-c".to_string(), "pwd".to_string()],
            cleanup_path: None,
            sandboxed: false,
            sandbox_backend: "direct".to_string(),
            sandbox_home: None,
        };
        let env_map = command_env_for_wrapped(
            &wrapped,
            &HashMap::from([("HOME".to_string(), "/real/home".to_string())]),
        )
        .expect("direct env");

        assert_eq!(env_map.get("HOME").map(String::as_str), Some("/real/home"));
        assert!(
            !env_map.contains_key("XDG_CONFIG_HOME"),
            "direct fallback should not apply sandbox-only XDG roots"
        );
    }

    #[test]
    fn unsandboxed_warning_names_the_backend() {
        let wrapped = WrappedCommand {
            program: "sh".to_string(),
            args: vec!["-c".to_string(), "pwd".to_string()],
            cleanup_path: None,
            sandboxed: false,
            sandbox_backend: "direct".to_string(),
            sandbox_home: None,
        };

        let warning = unsandboxed_execution_warning(&wrapped);

        assert!(warning.contains("without sandbox isolation"));
        assert!(warning.contains("direct"));
    }

    #[tokio::test]
    async fn streaming_call_reports_stdout_and_stderr_chunks() {
        let temp = tempdir().expect("tempdir");
        let sandbox = SandboxManager::new_for_rara_dir(temp.path().join(".rara")).expect("sandbox");
        let Ok(wrapped) = sandbox.wrap_exec_command(
            "/bin/sh",
            &[
                "-c".to_string(),
                "printf 'out\\n'; printf 'err\\n' >&2".to_string(),
            ],
            temp.path().to_string_lossy().as_ref(),
            false,
        ) else {
            return;
        };
        if !binary_exists(&wrapped.program) {
            return;
        }
        let tool = BashTool {
            sandbox: Arc::new(sandbox),
            background_tasks: Arc::new(
                BackgroundTaskStore::new(temp.path().join(".rara/background-tasks"))
                    .expect("background task store"),
            ),
        };
        let mut events = Vec::new();
        let result = tool
            .call_with_events(
                json!({
                    "program": "/bin/sh",
                    "args": ["-c", "printf 'out\\n'; printf 'err\\n' >&2"],
                }),
                &mut |event| events.push(event),
            )
            .await
            .expect("bash result");

        assert!(
            !events.is_empty(),
            "expected streamed events, got result: {result}"
        );
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
        assert_eq!(
            result.get("sandboxed").and_then(Value::as_bool),
            Some(wrapped.sandboxed)
        );
        assert_eq!(
            result.get("sandbox_backend").and_then(Value::as_str),
            Some(wrapped.sandbox_backend.as_str())
        );
    }

    #[tokio::test]
    async fn background_call_returns_task_and_status_reads_output() {
        let temp = tempdir().expect("tempdir");
        let sandbox = SandboxManager::new_for_rara_dir(temp.path().join(".rara")).expect("sandbox");
        let Ok(wrapped) = sandbox.wrap_exec_command(
            "sh",
            &["-c".to_string(), "printf 'background-out\\n'".to_string()],
            temp.path().to_string_lossy().as_ref(),
            false,
        ) else {
            return;
        };
        if !binary_exists(&wrapped.program) {
            return;
        }

        let background_tasks = Arc::new(
            BackgroundTaskStore::new(temp.path().join(".rara/background-tasks"))
                .expect("background task store"),
        );
        let tool = BashTool {
            sandbox: Arc::new(sandbox),
            background_tasks: background_tasks.clone(),
        };
        let status_tool = BackgroundTaskStatusTool {
            background_tasks: background_tasks.clone(),
        };

        let started = tool
            .call(json!({
                "program": "sh",
                "args": ["-c", "printf 'background-out\\n'"],
                "run_in_background": true,
            }))
            .await
            .expect("background start");
        let task_id = started
            .get("background_task_id")
            .and_then(Value::as_str)
            .expect("task id");
        assert_eq!(started.get("exit_code"), Some(&Value::Null));
        assert_eq!(
            started.get("status"),
            Some(&json!(BackgroundTaskStatus::Running))
        );

        let mut last = Value::Null;
        for _ in 0..50 {
            last = status_tool
                .call(json!({ "task_id": task_id, "tail_bytes": 4096 }))
                .await
                .expect("background status");
            if last.get("status") != Some(&json!(BackgroundTaskStatus::Running)) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        assert_ne!(
            last.get("status"),
            Some(&json!(BackgroundTaskStatus::Running))
        );
        assert!(last.get("output_path").and_then(Value::as_str).is_some());
    }

    #[tokio::test]
    async fn background_tasks_can_be_listed_and_stopped_without_count_limit() {
        let temp = tempdir().expect("tempdir");
        let sandbox = SandboxManager::new_for_rara_dir(temp.path().join(".rara")).expect("sandbox");
        let Ok(wrapped) = sandbox.wrap_exec_command(
            "sh",
            &["-c".to_string(), "sleep 30".to_string()],
            temp.path().to_string_lossy().as_ref(),
            false,
        ) else {
            return;
        };
        if !binary_exists(&wrapped.program) {
            return;
        }

        let background_tasks = Arc::new(
            BackgroundTaskStore::new(temp.path().join(".rara/background-tasks"))
                .expect("background task store"),
        );
        let tool = BashTool {
            sandbox: Arc::new(sandbox),
            background_tasks: background_tasks.clone(),
        };
        let list_tool = BackgroundTaskListTool {
            background_tasks: background_tasks.clone(),
        };
        let stop_tool = BackgroundTaskStopTool {
            background_tasks: background_tasks.clone(),
        };

        let started = tool
            .call(json!({
                "program": "sh",
                "args": ["-c", "sleep 30"],
                "run_in_background": true,
            }))
            .await
            .expect("background start");
        let task_id = started
            .get("background_task_id")
            .and_then(Value::as_str)
            .expect("task id")
            .to_string();

        let listed = list_tool.call(json!({})).await.expect("list tasks");
        assert_eq!(
            listed.get("tasks").and_then(Value::as_array).map(Vec::len),
            Some(1)
        );

        let stopped = stop_tool
            .call(json!({ "task_id": task_id }))
            .await
            .expect("stop task");
        assert_eq!(
            stopped.pointer("/stopped/0/status"),
            Some(&json!(BackgroundTaskStatus::Killed))
        );
    }

    #[tokio::test]
    async fn read_output_tail_returns_only_requested_suffix() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("task.log");
        tokio::fs::write(&path, b"0123456789tail")
            .await
            .expect("write log");

        let output = read_output_tail(&path, 4).await.expect("tail");

        assert_eq!(output, "tail");
    }

    #[tokio::test]
    async fn read_output_tail_missing_file_is_empty() {
        let temp = tempdir().expect("tempdir");

        let output = read_output_tail(&temp.path().join("missing.log"), 4)
            .await
            .expect("missing tail");

        assert_eq!(output, "");
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
