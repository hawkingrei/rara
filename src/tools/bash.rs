use crate::sandbox::{SandboxManager, WrappedCommand, sandbox_failure_hint};
use crate::tool::{Tool, ToolError, ToolOutputStream, ToolProgressEvent};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use tokio::fs;
use tokio::process::Command;
use tokio::sync::mpsc;

mod commands;
mod background;

pub use self::background::{
    BackgroundTaskListTool, BackgroundTaskStatusTool, BackgroundTaskStatus,
    BackgroundTaskStopTool, BackgroundTaskStore, BackgroundTaskRecord,
    read_output_tail, spawn_background_bash_task, read_stream_chunks,
};
pub use self::commands::{
    BashCommandInput, BashStreamKind,
    sandbox_command_env, command_env_for_wrapped,
    sandbox_output_hint, unsandboxed_execution_warning, ensure_sandbox_home_dirs,
};

pub struct BashTool {
    pub sandbox: Arc<SandboxManager>,
    pub background_tasks: Arc<BackgroundTaskStore>,
    pub base_env: Arc<HashMap<String, String>>,
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
        let command_env = command_env_for_wrapped(&wrapped, &self.base_env, &request.env)?;

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

mod tests {
    use super::{
        BackgroundTaskListTool, BackgroundTaskStatus, BackgroundTaskStatusTool,
        BackgroundTaskStopTool, BackgroundTaskStore, BashCommandInput, BashTool,
        command_env_for_wrapped, read_output_tail, sandbox_command_env, sandbox_output_hint,
        unsandboxed_execution_warning,
    };
    use crate::sandbox::{SandboxManager, WrappedCommand};
    use crate::tool::{Tool, ToolOutputStream, ToolProgressEvent};
    use serde_json::{Value, json};
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
    fn classifies_read_only_commands_for_approval_policy() {
        for command in [
            "git status --short",
            "git diff -- src/tools/bash.rs",
            "git log --oneline -n 5",
            "rg -n read_only src",
            "find src -name '*.rs'",
            "sed -n '1,20p' src/tools/bash.rs",
            "cat Cargo.toml | grep '^name'",
            "docker inspect rara-dev",
            "pyright --outputjson",
        ] {
            let input =
                BashCommandInput::from_value(json!({ "command": command })).expect("bash payload");
            assert!(input.is_read_only(), "{command} should be read-only");
        }
    }

    #[test]
    fn keeps_write_network_background_and_complex_commands_under_approval() {
        for payload in [
            json!({ "command": "git push origin main" }),
            json!({ "command": "rm -rf target" }),
            json!({ "command": "sed -i '' 's/a/b/' Cargo.toml" }),
            json!({ "command": "find . -name '*.tmp' -delete" }),
            json!({ "command": "cat Cargo.toml > /tmp/out" }),
            json!({ "command": "git status", "allow_net": true }),
            json!({ "command": "rg TODO", "run_in_background": true }),
            json!({ "program": "rg", "args": ["TODO"], "env": { "PATH": "/tmp/bin" } }),
        ] {
            let input = BashCommandInput::from_value(payload).expect("bash payload");
            assert!(
                !input.is_read_only(),
                "{} should require approval",
                input.summary()
            );
        }
    }

    #[test]
    fn classifies_structured_read_only_programs() {
        let input = BashCommandInput::from_value(json!({
            "program": "/usr/bin/git",
            "args": ["status", "--short"]
        }))
        .expect("structured payload");

        assert!(input.is_read_only());
    }

    #[test]
    fn derives_and_matches_codex_style_approval_prefix() {
        let input = BashCommandInput::from_value(json!({
            "command": "git push origin main"
        }))
        .expect("bash payload");

        assert_eq!(input.approval_prefix().as_deref(), Some("git push"));
        assert!(input.matches_approval_prefix("git push"));
        assert!(!input.matches_approval_prefix("git pull"));
    }

    #[test]
    fn approval_prefix_matching_normalizes_program_paths() {
        let shell_input = BashCommandInput::from_value(json!({
            "command": "/usr/bin/git push origin main"
        }))
        .expect("shell payload");
        assert_eq!(shell_input.approval_prefix().as_deref(), Some("git push"));
        assert!(shell_input.matches_approval_prefix("git push"));

        let structured_input = BashCommandInput::from_value(json!({
            "program": "/usr/bin/git",
            "args": ["push", "origin", "main"]
        }))
        .expect("structured payload");
        assert_eq!(
            structured_input.approval_prefix().as_deref(),
            Some("git push")
        );
        assert!(structured_input.matches_approval_prefix("git push"));
    }

    #[test]
    fn approval_prefix_skips_known_global_options() {
        let input = BashCommandInput::from_value(json!({
            "command": "git --no-pager push origin main"
        }))
        .expect("shell payload");

        assert_eq!(input.approval_prefix().as_deref(), Some("git push"));
        assert!(input.matches_approval_prefix("git push"));
    }

    #[test]
    fn sandbox_command_env_defaults_home_and_xdg_roots() {
        let sandbox_home = Path::new("/tmp/rara-test-home");
        let base_env = HashMap::from([("PATH".to_string(), "/custom/bin:/usr/bin".to_string())]);
        let env_map = sandbox_command_env(sandbox_home, &base_env, &HashMap::new());

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
    }

    #[test]
    fn sandbox_command_env_keeps_explicit_overrides() {
        let sandbox_home = Path::new("/tmp/rara-test-home");
        let env_map = sandbox_command_env(
            sandbox_home,
            &HashMap::from([("PATH".to_string(), "/snapshot/bin".to_string())]),
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
    fn sandbox_command_env_falls_back_to_process_path_when_snapshot_path_is_missing() {
        let sandbox_home = Path::new("/tmp/rara-test-home");
        let env_map = sandbox_command_env(
            sandbox_home,
            &HashMap::from([("PATH".to_string(), String::new())]),
            &HashMap::new(),
        );

        assert!(
            env_map.get("PATH").is_some_and(|path| !path.is_empty()),
            "sandbox env must keep a usable PATH after env_clear"
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
            &HashMap::from([("PATH".to_string(), "/snapshot/bin".to_string())]),
            &HashMap::from([("HOME".to_string(), "/real/home".to_string())]),
        )
        .expect("direct env");

        assert_eq!(env_map.get("HOME").map(String::as_str), Some("/real/home"));
        assert_eq!(
            env_map.get("PATH").map(String::as_str),
            Some("/snapshot/bin")
        );
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
            base_env: Arc::new(HashMap::new()),
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
            base_env: Arc::new(HashMap::new()),
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
            base_env: Arc::new(HashMap::new()),
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
