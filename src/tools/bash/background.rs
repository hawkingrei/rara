use crate::sandbox::{SandboxManager, WrappedCommand, sandbox_failure_hint};
use crate::tool::{Tool, ToolError, ToolOutputStream, ToolProgressEvent};
use async_trait::async_trait;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::SeekFrom;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use tokio::fs;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::process::Child;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use uuid::Uuid;

use super::commands::{
    BashStreamKind, sandbox_command_env, command_env_for_wrapped,
    sandbox_output_hint, unsandboxed_execution_warning, ensure_sandbox_home_dirs,
};

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

