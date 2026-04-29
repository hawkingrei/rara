use crate::sandbox::{WrappedCommand, sandbox_failure_hint};
use crate::tool::{ToolError, ToolOutputStream};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::env;
use std::path::Path;
use tokio::fs;

pub(crate) enum BashStreamKind {
    Stdout,
    Stderr,
}

impl BashStreamKind {
    pub(crate) fn output_stream(self) -> ToolOutputStream {
        match self {
            Self::Stdout => ToolOutputStream::Stdout,
            Self::Stderr => ToolOutputStream::Stderr,
        }
    }
}


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

    pub fn is_read_only(&self) -> bool {
        if self.allow_net || self.run_in_background || !self.env.is_empty() {
            return false;
        }
        if let Some(command) = self
            .command
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            return shell_command_is_read_only(command);
        }
        self.program
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .is_some_and(|program| argv_is_read_only(program, &self.args))
    }

    pub fn approval_prefix(&self) -> Option<String> {
        if let Some(command) = self
            .command
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            let segments = split_shell_segments(command)?;
            if segments.len() != 1 {
                return None;
            }
            let tokens = tokenize_shell_segment(&segments[0])?;
            return prefix_from_tokens(&tokens);
        }

        let program = self
            .program
            .as_deref()
            .filter(|value| !value.trim().is_empty())?;
        let mut tokens = Vec::with_capacity(self.args.len() + 1);
        tokens.push(program.to_string());
        tokens.extend(self.args.iter().cloned());
        prefix_from_tokens(&tokens)
    }

    pub fn matches_approval_prefix(&self, prefix: &str) -> bool {
        let normalized = self.normalized_approval_summary();
        normalized == prefix
            || normalized
                .strip_prefix(prefix)
                .is_some_and(|suffix| suffix.starts_with(char::is_whitespace))
    }

    fn normalized_approval_summary(&self) -> String {
        if let Some(command) = self
            .command
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            if let Some(tokens) = split_shell_segments(command).and_then(|segments| {
                if segments.len() == 1 {
                    tokenize_shell_segment(&segments[0])
                } else {
                    None
                }
            }) {
                return normalized_tokens_summary(&tokens);
            }
            return command.to_string();
        }

        let Some(program) = self
            .program
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        else {
            return self.summary();
        };
        let mut tokens = Vec::with_capacity(self.args.len() + 1);
        tokens.push(program.to_string());
        tokens.extend(self.args.iter().cloned());
        normalized_tokens_summary(&tokens)
    }
}


fn prefix_from_tokens(tokens: &[String]) -> Option<String> {
    let program = tokens.first()?;
    let program = command_basename(program);
    if let Some(subcommand) = approval_subcommand_token(program, &tokens[1..]) {
        Some(format!("{program} {subcommand}"))
    } else {
        Some(program.to_string())
    }
}

fn normalized_tokens_summary(tokens: &[String]) -> String {
    let Some(program) = tokens.first() else {
        return String::new();
    };
    let program = command_basename(program);
    let rest = &tokens[1..];
    let args = approval_subcommand_index(program, rest)
        .map(|index| rest[index..].iter().cloned().collect::<Vec<_>>())
        .unwrap_or_else(|| rest.to_vec());
    std::iter::once(program.to_string())
        .chain(args)
        .collect::<Vec<_>>()
        .join(" ")
}

fn command_basename(command: &str) -> &str {
    Path::new(command)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(command)
}

fn approval_subcommand_token<'a>(program: &str, args: &'a [String]) -> Option<&'a str> {
    approval_subcommand_index(program, args).and_then(|index| args.get(index).map(String::as_str))
}

fn approval_subcommand_index(program: &str, args: &[String]) -> Option<usize> {
    match program {
        "git" => skip_known_global_options(
            args,
            &["--no-pager", "--no-optional-locks"],
            &["-C", "-c", "--git-dir", "--work-tree"],
        ),
        "docker" => skip_known_global_options(
            args,
            &["--debug", "--tls", "--tlsverify"],
            &["--config", "--context", "--host", "-H", "--log-level"],
        ),
        _ => args.first().map(|_| 0),
    }
}

fn skip_known_global_options(
    args: &[String],
    valueless_options: &[&str],
    value_options: &[&str],
) -> Option<usize> {
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        if valueless_options.contains(&arg) {
            index += 1;
        } else if value_options.contains(&arg) {
            index += 2;
        } else if value_options
            .iter()
            .any(|option| arg.starts_with(&format!("{option}=")))
        {
            index += 1;
        } else if arg.starts_with('-') {
            index += 1;
        } else {
            return Some(index);
        }
    }
    None
}

fn shell_command_is_read_only(command: &str) -> bool {
    if command.contains('\n')
        || command.contains('`')
        || command.contains("$(")
        || command.contains('>')
    {
        return false;
    }
    split_shell_segments(command)
        .filter(|segments| !segments.is_empty())
        .is_some_and(|segments| {
            segments.into_iter().all(|segment| {
                tokenize_shell_segment(&segment).is_some_and(|tokens| {
                    if tokens.is_empty() {
                        return false;
                    }
                    argv_is_read_only(&tokens[0], &tokens[1..])
                })
            })
        })
}

fn argv_is_read_only(program: &str, args: &[String]) -> bool {
    let program = Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(program);
    match program {
        "pwd" | "ls" | "tree" | "cat" | "head" | "tail" | "wc" | "stat" | "file" | "du" | "df"
        | "which" | "type" | "whereis" | "uname" => true,
        "rg" | "grep" => !args
            .iter()
            .any(|arg| matches!(arg.as_str(), "--files-with-matches=")),
        "sed" => !args.iter().any(|arg| {
            arg == "-i"
                || arg.starts_with("-i.")
                || arg == "--in-place"
                || arg.starts_with("--in-place=")
        }),
        "find" => !args.iter().any(|arg| {
            matches!(
                arg.as_str(),
                "-delete" | "-exec" | "-execdir" | "-ok" | "-okdir"
            )
        }),
        "fd" | "fdfind" => !args.iter().any(|arg| {
            matches!(
                arg.as_str(),
                "-x" | "--exec" | "-X" | "--exec-batch" | "--list-details"
            )
        }),
        "git" => git_args_are_read_only(args),
        "docker" => docker_args_are_read_only(args),
        "pyright" => !args
            .iter()
            .any(|arg| matches!(arg.as_str(), "--watch" | "-w")),
        _ => false,
    }
}

fn git_args_are_read_only(args: &[String]) -> bool {
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--no-pager" | "--no-optional-locks" => index += 1,
            "-C" | "-c" | "--git-dir" | "--work-tree" => return false,
            value if value.starts_with('-') => return false,
            _ => break,
        }
    }
    let Some(subcommand) = args.get(index).map(String::as_str) else {
        return false;
    };
    let rest = &args[index + 1..];
    match subcommand {
        "diff" | "log" | "show" | "shortlog" | "status" | "blame" | "ls-files" | "merge-base"
        | "rev-parse" | "rev-list" | "describe" | "cat-file" | "for-each-ref" | "grep" => true,
        "stash" => rest.first().is_some_and(|value| value == "list"),
        "remote" => rest.is_empty() || rest == ["-v"] || rest == ["--verbose"],
        "config" => rest.first().is_some_and(|value| value == "--get"),
        "reflog" => !rest
            .iter()
            .any(|value| matches!(value.as_str(), "expire" | "delete" | "exists")),
        "branch" => {
            rest.is_empty()
                || rest.iter().all(|value| {
                    matches!(
                        value.as_str(),
                        "--list" | "-l" | "-a" | "--all" | "-r" | "--remotes" | "-v" | "-vv"
                    )
                })
        }
        _ => false,
    }
}

fn docker_args_are_read_only(args: &[String]) -> bool {
    args.first()
        .is_some_and(|value| matches!(value.as_str(), "ps" | "images" | "logs" | "inspect"))
}

fn split_shell_segments(command: &str) -> Option<Vec<String>> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut chars = command.chars().peekable();
    let mut quote: Option<char> = None;
    while let Some(ch) = chars.next() {
        if let Some(active_quote) = quote {
            if ch == active_quote {
                quote = None;
            }
            current.push(ch);
            continue;
        }
        match ch {
            '\'' | '"' => {
                quote = Some(ch);
                current.push(ch);
            }
            ';' | '|' => {
                push_shell_segment(&mut segments, &mut current);
            }
            '&' if chars.peek() == Some(&'&') => {
                chars.next();
                push_shell_segment(&mut segments, &mut current);
            }
            '&' => return None,
            _ => current.push(ch),
        }
    }
    if quote.is_some() {
        return None;
    }
    push_shell_segment(&mut segments, &mut current);
    Some(segments)
}

fn push_shell_segment(segments: &mut Vec<String>, current: &mut String) {
    let segment = current.trim();
    if !segment.is_empty() {
        segments.push(segment.to_string());
    }
    current.clear();
}

fn tokenize_shell_segment(segment: &str) -> Option<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = segment.chars().peekable();
    let mut quote: Option<char> = None;
    while let Some(ch) = chars.next() {
        match quote {
            Some(active_quote) => {
                if ch == active_quote {
                    quote = None;
                } else if ch == '\\' && active_quote == '"' {
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                } else {
                    current.push(ch);
                }
            }
            None => match ch {
                '\'' | '"' => quote = Some(ch),
                '\\' => {
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                }
                '<' => return None,
                value if value.is_whitespace() => {
                    if !current.is_empty() {
                        tokens.push(std::mem::take(&mut current));
                    }
                }
                _ => current.push(ch),
            },
        }
    }
    if quote.is_some() {
        return None;
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Some(tokens)
}

pub(crate) pub(crate) fn sandbox_command_env(
    sandbox_home: &Path,
    base_env: &HashMap<String, String>,
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
    env_map.extend(
        base_env
            .iter()
            .map(|(key, value)| (key.clone(), value.clone())),
    );
    env_map.extend(
        overrides
            .iter()
            .map(|(key, value)| (key.clone(), value.clone())),
    );
    ensure_usable_path(&mut env_map);
    env_map
}

fn ensure_usable_path(env_map: &mut HashMap<String, String>) {
    let needs_path = env_map.get("PATH").map_or(true, |value| value.is_empty());
    if needs_path {
        let fallback_path = env::var("PATH")
            .ok()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "/usr/bin:/bin".to_string());
        env_map.insert("PATH".to_string(), fallback_path);
    }
}

pub(crate) pub(crate) fn command_env_for_wrapped(
    wrapped: &WrappedCommand,
    base_env: &HashMap<String, String>,
    overrides: &HashMap<String, String>,
) -> Result<HashMap<String, String>, ToolError> {
    if wrapped.sandboxed {
        let sandbox_home = wrapped.sandbox_home.as_deref().ok_or_else(|| {
            ToolError::ExecutionFailed("sandboxed command is missing sandbox home".into())
        })?;
        Ok(sandbox_command_env(sandbox_home, base_env, overrides))
    } else {
        let mut env_map = HashMap::with_capacity(base_env.len() + overrides.len());
        env_map.extend(
            base_env
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );
        env_map.extend(
            overrides
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );
        Ok(env_map)
    }
}


pub(crate) pub(crate) fn sandbox_output_hint(stderr: &str) -> Option<&'static str> {
    let lower = stderr.to_ascii_lowercase();
    if lower.contains("sandbox: violation")
        || lower.contains("operation not permitted")
        || lower.contains("command not found")
        || lower.contains("no such file or directory")
        || lower.contains("permission denied")
    {
        Some(
            "\n\nhint: Sandboxed bash appears blocked or missing a runtime path. Prefer direct file tools such as read_file, apply_patch, and replace_lines; ask the user only if a real shell command is required.\n",
        )
    } else {
        None
    }
}

pub(crate) pub(crate) fn unsandboxed_execution_warning(wrapped: &WrappedCommand) -> String {
    format!(
        "warning: command is running without sandbox isolation (backend: {}).\n",
        wrapped.sandbox_backend
    )
}

pub(crate) pub(crate) async fn ensure_sandbox_home_dirs(sandbox_home: &Path) -> Result<(), ToolError> {
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

