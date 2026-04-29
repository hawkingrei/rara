use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Result, bail};
use uuid::Uuid;

pub struct SandboxManager {
    os: String,
    profile_dir: PathBuf,
    sandbox_home: PathBuf,
    backend: SandboxBackend,
    command_install_roots: Vec<PathBuf>,
}

#[derive(Debug)]
pub struct WrappedCommand {
    pub program: String,
    pub args: Vec<String>,
    pub cleanup_path: Option<PathBuf>,
    pub sandboxed: bool,
    pub sandbox_backend: String,
    pub sandbox_home: Option<PathBuf>,
}

const LINUX_RUNTIME_READ_ROOTS: &[&str] = &[
    "/bin",
    "/sbin",
    "/usr",
    "/opt",
    "/etc",
    "/lib",
    "/lib64",
    "/nix/store",
    "/run/current-system/sw",
];
const MACOS_SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";
const DEFAULT_SHELL: &str = "/bin/sh";
const PROFILE_CLEANUP_AGE: Duration = Duration::from_secs(60 * 60);

#[derive(Clone, Debug, PartialEq, Eq)]
enum SandboxBackend {
    MacosSeatbelt,
    LinuxBubblewrap,
    Direct,
    Unsupported { platform: String },
}

impl SandboxManager {
    pub fn new() -> Result<Self> {
        let os = std::env::consts::OS.to_string();
        let root = std::env::current_dir()?;
        let rara_dir = rara_config::workspace_data_dir_for(&root)?;
        Self::new_with_rara_dir(os, rara_dir)
    }

    pub fn new_for_rara_dir(rara_dir: PathBuf) -> Result<Self> {
        let os = std::env::consts::OS.to_string();
        Self::new_with_rara_dir(os, rara_dir)
    }

    pub fn new_with_command_path(command_path: Option<String>) -> Result<Self> {
        let os = std::env::consts::OS.to_string();
        let root = std::env::current_dir()?;
        let rara_dir = rara_config::workspace_data_dir_for(&root)?;
        Self::new_with_rara_dir_and_command_path(os, rara_dir, command_path.map(OsString::from))
    }

    fn new_with_rara_dir(os: String, rara_dir: PathBuf) -> Result<Self> {
        Self::new_with_rara_dir_and_command_path(os, rara_dir, env::var_os("PATH"))
    }

    fn new_with_rara_dir_and_command_path(
        os: String,
        rara_dir: PathBuf,
        command_path: Option<OsString>,
    ) -> Result<Self> {
        if !rara_dir.exists() {
            fs::create_dir_all(&rara_dir)?;
        }
        let profile_dir = rara_dir.join("sandbox");
        if !profile_dir.exists() {
            fs::create_dir_all(&profile_dir)?;
        }
        cleanup_stale_profiles(&profile_dir)?;
        let sandbox_home = process_sandbox_home();

        let backend = SandboxBackend::detect(os.as_str());
        let command_install_roots = command_search_install_roots(command_path.as_deref());

        Ok(Self {
            os,
            profile_dir,
            sandbox_home,
            backend,
            command_install_roots,
        })
    }

    fn create_profile(&self, allow_net: bool) -> Result<PathBuf> {
        if self.os != "macos" {
            bail!("sandbox profile creation is only supported on macOS");
        }

        let mut file_rules = String::new();
        if let Some(home) = env::var_os("HOME").map(PathBuf::from) {
            for sensitive_dir in [".ssh", ".aws"] {
                let path = home.join(sensitive_dir);
                file_rules.push_str(&format!(
                    "(deny file-read* (subpath \"{}\"))\n",
                    sandbox_profile_string_literal(&path)
                ));
            }
        }
        for root in &self.command_install_roots {
            let root = sandbox_profile_string_literal(&root);
            file_rules.push_str(&format!(
                "(allow file-read* (subpath \"{root}\"))\n(allow file-map-executable (subpath \"{root}\"))\n"
            ));
        }

        let mut net_rules = String::new();
        if allow_net {
            net_rules.push_str("(allow network*)");
        } else {
            net_rules.push_str("(deny network*)\n(allow network-outbound (literal \"/private/var/run/mDNSResponder\"))\n");
        }

        let profile = format!(
            r#"(version 1)
(deny default)
{}
(allow file-read* (subpath "/usr/bin"))
(allow file-read* (subpath "/bin"))
(allow file-read* (subpath "/System"))
(allow file-read* (subpath (param "CWD")))
(allow file-write* (subpath (param "CWD")))
(allow file-write* (subpath "/tmp"))
(allow process*)
(allow sysctl-read)
{}
"#,
            file_rules, net_rules
        );
        let profile_path = self
            .profile_dir
            .join(format!("sandbox-{}.sb", Uuid::new_v4()));
        fs::write(&profile_path, profile)?;
        Ok(profile_path)
    }

    fn append_mount_target_parent_dirs(args: &mut Vec<String>, mount_target: &Path) {
        let Some(mount_target_dir) = mount_target.parent() else {
            return;
        };

        let mut mount_target_dirs: Vec<PathBuf> = mount_target_dir
            .ancestors()
            .take_while(|path| path != &Path::new("/"))
            .map(Path::to_path_buf)
            .collect();
        mount_target_dirs.reverse();
        for mount_target_dir in mount_target_dirs {
            args.push("--dir".to_string());
            args.push(mount_target_dir.display().to_string());
        }
    }

    fn append_ro_bind(args: &mut Vec<String>, path: &Path) {
        Self::append_mount_target_parent_dirs(args, path);
        args.push("--ro-bind".to_string());
        args.push(path.display().to_string());
        args.push(path.display().to_string());
    }

    fn linux_runtime_read_roots(&self) -> Vec<PathBuf> {
        let mut roots = LINUX_RUNTIME_READ_ROOTS
            .iter()
            .map(PathBuf::from)
            .filter(|path| path.exists())
            .collect::<Vec<_>>();
        for path_dir in &self.command_install_roots {
            if roots.iter().any(|root| path_dir.starts_with(root)) {
                continue;
            }
            roots.push(path_dir.clone());
        }
        roots
    }

    fn linux_sandbox_args(&self, cwd: &str, allow_net: bool) -> Vec<String> {
        let cwd_path = Path::new(cwd);
        let mut args = vec![
            "--new-session".to_string(),
            "--die-with-parent".to_string(),
            "--tmpfs".to_string(),
            "/".to_string(),
            "--dev".to_string(),
            "/dev".to_string(),
            "--proc".to_string(),
            "/proc".to_string(),
            "--tmpfs".to_string(),
            "/tmp".to_string(),
            "--dir".to_string(),
            "/run".to_string(),
            "--dir".to_string(),
            "/var".to_string(),
            "--symlink".to_string(),
            "../run".to_string(),
            "/var/run".to_string(),
        ];
        for dir in sandbox_home_dirs(&self.sandbox_home) {
            args.push("--dir".to_string());
            args.push(dir.display().to_string());
        }

        for root in self.linux_runtime_read_roots() {
            Self::append_ro_bind(&mut args, &root);
        }

        if let Ok(resolv_conf) = fs::metadata("/etc/resolv.conf") {
            if resolv_conf.is_file() {
                Self::append_ro_bind(&mut args, Path::new("/etc/resolv.conf"));
            }
        }

        Self::append_mount_target_parent_dirs(&mut args, cwd_path);
        args.push("--bind".to_string());
        args.push(cwd.to_string());
        args.push(cwd.to_string());
        args.push("--chdir".to_string());
        args.push(cwd.to_string());

        if !allow_net {
            args.push("--unshare-net".to_string());
        }

        args
    }

    pub fn wrap_shell_command(
        &self,
        original_cmd: &str,
        cwd: &str,
        allow_net: bool,
    ) -> Result<WrappedCommand> {
        let shell = shell_program();
        let shell_flag = shell_command_flag(&shell);
        match &self.backend {
            SandboxBackend::MacosSeatbelt => {
                let profile_path = self.create_profile(allow_net)?;
                Ok(WrappedCommand {
                    program: MACOS_SANDBOX_EXEC.to_string(),
                    args: vec![
                        "-D".to_string(),
                        format!("CWD={cwd}"),
                        "-f".to_string(),
                        profile_path.display().to_string(),
                        shell,
                        shell_flag,
                        original_cmd.to_string(),
                    ],
                    cleanup_path: Some(profile_path),
                    sandboxed: true,
                    sandbox_backend: self.backend.name().to_string(),
                    sandbox_home: Some(self.sandbox_home.clone()),
                })
            }
            SandboxBackend::LinuxBubblewrap => {
                let mut args = self.linux_sandbox_args(cwd, allow_net);
                args.push("--".to_string());
                args.push(shell);
                args.push(shell_flag);
                args.push(original_cmd.to_string());
                Ok(WrappedCommand {
                    program: "bwrap".to_string(),
                    args,
                    cleanup_path: None,
                    sandboxed: true,
                    sandbox_backend: self.backend.name().to_string(),
                    sandbox_home: Some(self.sandbox_home.clone()),
                })
            }
            SandboxBackend::Direct => Ok(wrap_direct_shell_command(original_cmd)),
            SandboxBackend::Unsupported { platform } => bail!(
                "sandboxed command execution is unsupported on platform {}",
                platform
            ),
        }
    }

    pub fn wrap_pty_shell_command(
        &self,
        original_cmd: &str,
        cwd: &str,
        allow_net: bool,
    ) -> Result<WrappedCommand> {
        match &self.backend {
            // macOS sandbox-exec does not preserve interactive PTY stdin reliably.
            SandboxBackend::MacosSeatbelt => Ok(wrap_direct_shell_command(original_cmd)),
            _ => self.wrap_shell_command(original_cmd, cwd, allow_net),
        }
    }

    pub fn wrap_exec_command(
        &self,
        program: &str,
        args: &[String],
        cwd: &str,
        allow_net: bool,
    ) -> Result<WrappedCommand> {
        match &self.backend {
            SandboxBackend::MacosSeatbelt => {
                let profile_path = self.create_profile(allow_net)?;
                let mut wrapped_args = vec![
                    "-D".to_string(),
                    format!("CWD={cwd}"),
                    "-f".to_string(),
                    profile_path.display().to_string(),
                    program.to_string(),
                ];
                wrapped_args.extend(args.iter().cloned());
                Ok(WrappedCommand {
                    program: MACOS_SANDBOX_EXEC.to_string(),
                    args: wrapped_args,
                    cleanup_path: Some(profile_path),
                    sandboxed: true,
                    sandbox_backend: self.backend.name().to_string(),
                    sandbox_home: Some(self.sandbox_home.clone()),
                })
            }
            SandboxBackend::LinuxBubblewrap => {
                let mut wrapped_args = self.linux_sandbox_args(cwd, allow_net);
                wrapped_args.push("--".to_string());
                wrapped_args.push(program.to_string());
                wrapped_args.extend(args.iter().cloned());
                Ok(WrappedCommand {
                    program: "bwrap".to_string(),
                    args: wrapped_args,
                    cleanup_path: None,
                    sandboxed: true,
                    sandbox_backend: self.backend.name().to_string(),
                    sandbox_home: Some(self.sandbox_home.clone()),
                })
            }
            SandboxBackend::Direct => Ok(wrap_direct_exec_command(program, args)),
            SandboxBackend::Unsupported { platform } => bail!(
                "sandboxed command execution is unsupported on platform {}",
                platform
            ),
        }
    }

    pub fn explain_violation(&self, stderr: &str) -> Option<String> {
        if stderr.contains("Operation not permitted") || stderr.contains("Sandbox: Violation") {
            Some("Blocked by RARA Sandbox.".into())
        } else {
            None
        }
    }
}

pub fn sandbox_failure_hint() -> &'static str {
    "Sandboxed bash could not complete this command. Prefer direct file tools such as read_file, apply_patch, and replace_lines; if shell access is required, ask the user to run or approve a shell-specific path."
}

impl SandboxBackend {
    fn detect(os: &str) -> Self {
        match os {
            "macos" if Path::new(MACOS_SANDBOX_EXEC).is_file() => Self::MacosSeatbelt,
            "macos" => Self::Unsupported {
                platform: format!(
                    "macos (sandbox unavailable: {} is missing)",
                    MACOS_SANDBOX_EXEC
                ),
            },
            "linux" if command_exists("bwrap") => Self::LinuxBubblewrap,
            "linux" => Self::Unsupported {
                platform: "linux (sandbox unavailable: install bubblewrap/bwrap)".to_string(),
            },
            platform => Self::Unsupported {
                platform: platform.to_string(),
            },
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Self::MacosSeatbelt => "macos-seatbelt",
            Self::LinuxBubblewrap => "linux-bubblewrap",
            Self::Direct => "direct",
            Self::Unsupported { .. } => "unsupported",
        }
    }
}

fn wrap_direct_shell_command(original_cmd: &str) -> WrappedCommand {
    let shell = shell_program();
    let shell_flag = shell_command_flag(&shell);
    WrappedCommand {
        program: shell,
        args: vec![shell_flag, original_cmd.to_string()],
        cleanup_path: None,
        sandboxed: false,
        sandbox_backend: SandboxBackend::Direct.name().to_string(),
        sandbox_home: None,
    }
}

fn wrap_direct_exec_command(program: &str, args: &[String]) -> WrappedCommand {
    WrappedCommand {
        program: program.to_string(),
        args: args.to_vec(),
        cleanup_path: None,
        sandboxed: false,
        sandbox_backend: SandboxBackend::Direct.name().to_string(),
        sandbox_home: None,
    }
}

fn command_exists(program: &str) -> bool {
    let program_path = Path::new(program);
    if program_path.components().count() > 1 {
        return program_path.exists();
    }

    env::var_os("PATH")
        .map(|paths| env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
        .unwrap_or(false)
}

fn command_search_path_dirs(command_path: Option<&std::ffi::OsStr>) -> Vec<PathBuf> {
    command_path
        .map(OsString::from)
        .or_else(|| env::var_os("PATH"))
        .map(|paths| {
            let mut dirs = Vec::new();
            for dir in env::split_paths(&paths) {
                if !dir.is_absolute() || !dir.is_dir() {
                    continue;
                }
                if path_contains_control_chars(&dir) {
                    continue;
                }
                let dir = fs::canonicalize(&dir).unwrap_or(dir);
                if path_contains_control_chars(&dir) {
                    continue;
                }
                if !dirs.iter().any(|existing| existing == &dir) {
                    dirs.push(dir);
                }
            }
            dirs
        })
        .unwrap_or_default()
}

fn command_search_install_roots(command_path: Option<&std::ffi::OsStr>) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let home_dir = env::var_os("HOME")
        .map(PathBuf::from)
        .and_then(|path| fs::canonicalize(&path).ok().or(Some(path)));
    for dir in command_search_path_dirs(command_path) {
        let root = if matches!(
            dir.file_name().and_then(|name| name.to_str()),
            Some("bin" | "sbin")
        ) {
            let parent = dir.parent().unwrap_or_else(|| dir.as_path());
            if parent == Path::new("/") || home_dir.as_deref() == Some(parent) {
                dir.clone()
            } else {
                parent.to_path_buf()
            }
        } else {
            dir
        };
        if !roots.iter().any(|existing| existing == &root) {
            roots.push(root);
        }
    }
    roots
}

fn path_contains_control_chars(path: &Path) -> bool {
    path.display().to_string().chars().any(char::is_control)
}

fn sandbox_profile_string_literal(path: &Path) -> String {
    path.display()
        .to_string()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

fn shell_program() -> String {
    env::var("SHELL")
        .ok()
        .and_then(|value| sanitize_shell_program(value.as_str()))
        .unwrap_or_else(|| DEFAULT_SHELL.to_string())
}

fn sanitize_shell_program(value: &str) -> Option<String> {
    let shell = value.split_whitespace().next()?.trim();
    if matches!(
        shell,
        "/bin/sh"
            | "/bin/bash"
            | "/bin/zsh"
            | "/bin/ksh"
            | "/usr/bin/sh"
            | "/usr/bin/bash"
            | "/usr/bin/zsh"
            | "/usr/bin/ksh"
    ) {
        Some(shell.to_string())
    } else {
        None
    }
}

fn shell_command_flag(shell: &str) -> String {
    let name = Path::new(shell)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(shell);
    if matches!(name, "bash" | "zsh" | "ksh") {
        "-lc".to_string()
    } else {
        "-c".to_string()
    }
}

fn process_sandbox_home() -> PathBuf {
    PathBuf::from("/tmp").join(format!(
        "rara-home-{}-{}",
        std::process::id(),
        Uuid::new_v4()
    ))
}

fn sandbox_home_dirs(sandbox_home: &Path) -> Vec<PathBuf> {
    vec![
        sandbox_home.to_path_buf(),
        sandbox_home.join(".config"),
        sandbox_home.join(".cache"),
        sandbox_home.join(".local"),
        sandbox_home.join(".local/state"),
        sandbox_home.join(".local/share"),
    ]
}

fn cleanup_stale_profiles(profile_dir: &Path) -> Result<()> {
    cleanup_profiles_older_than(profile_dir, PROFILE_CLEANUP_AGE)
}

fn cleanup_profiles_older_than(profile_dir: &Path, max_age: Duration) -> Result<()> {
    let now = SystemTime::now();
    for entry in fs::read_dir(profile_dir)? {
        let entry = entry?;
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if path.is_file() && file_name.starts_with("sandbox-") && file_name.ends_with(".sb") {
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            let Ok(modified) = metadata.modified() else {
                continue;
            };
            let Ok(age) = now.duration_since(modified) else {
                continue;
            };
            if age >= max_age {
                let _ = fs::remove_file(&path);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_SHELL, MACOS_SANDBOX_EXEC, SandboxBackend, SandboxManager,
        cleanup_profiles_older_than, cleanup_stale_profiles, command_search_install_roots,
        sandbox_profile_string_literal, sanitize_shell_program, shell_command_flag, shell_program,
    };
    use std::env;
    use std::path::PathBuf;
    use std::time::Duration;
    use tempfile::tempdir;

    fn manager(os: &str, backend: SandboxBackend) -> SandboxManager {
        SandboxManager {
            os: os.to_string(),
            profile_dir: PathBuf::from("/tmp/rara-test-sandbox"),
            sandbox_home: PathBuf::from("/tmp/rara-test-sandbox-home"),
            backend,
            command_install_roots: command_search_install_roots(
                std::env::var_os("PATH").as_deref(),
            ),
        }
    }

    fn set_env_var<K, V>(key: K, value: V)
    where
        K: AsRef<std::ffi::OsStr>,
        V: AsRef<std::ffi::OsStr>,
    {
        // These tests restore PATH immediately after the scoped assertion.
        unsafe { std::env::set_var(key, value) };
    }

    fn remove_env_var<K>(key: K)
    where
        K: AsRef<std::ffi::OsStr>,
    {
        // These tests restore PATH immediately after the scoped assertion.
        unsafe { std::env::remove_var(key) };
    }

    #[test]
    fn wrap_command_fails_closed_on_unsupported_platform() {
        let manager = manager(
            "freebsd",
            SandboxBackend::Unsupported {
                platform: "freebsd".to_string(),
            },
        );

        let err = manager
            .wrap_shell_command("echo test", "/tmp", false)
            .expect_err("unsupported platforms should not fall back to unsandboxed execution");

        assert!(
            err.to_string()
                .contains("sandboxed command execution is unsupported on platform freebsd")
        );
    }

    #[test]
    fn detect_fails_closed_when_macos_sandbox_exec_is_unavailable() {
        let backend = SandboxBackend::detect("macos");

        if PathBuf::from(MACOS_SANDBOX_EXEC).is_file() {
            assert_eq!(backend, SandboxBackend::MacosSeatbelt);
        } else {
            assert!(matches!(
                backend,
                SandboxBackend::Unsupported { platform } if platform.contains("sandbox unavailable")
            ));
        }
    }

    #[test]
    fn detect_fails_closed_when_linux_bwrap_is_unavailable() {
        let original_path = std::env::var_os("PATH");
        set_env_var("PATH", "");
        let backend = SandboxBackend::detect("linux");
        if let Some(path) = original_path {
            set_env_var("PATH", path);
        } else {
            remove_env_var("PATH");
        }

        assert!(matches!(
            backend,
            SandboxBackend::Unsupported { platform } if platform.contains("install bubblewrap")
        ));
    }

    #[test]
    fn wrap_command_creates_unique_cleanup_profile_on_macos() {
        let tempdir = tempdir().expect("tempdir");
        let manager = SandboxManager {
            os: "macos".to_string(),
            profile_dir: tempdir.path().to_path_buf(),
            sandbox_home: tempdir.path().join("home"),
            backend: SandboxBackend::MacosSeatbelt,
            command_install_roots: command_search_install_roots(
                std::env::var_os("PATH").as_deref(),
            ),
        };

        let wrapped = manager
            .wrap_shell_command("echo test", "/tmp", false)
            .expect("macos sandbox wrapper");

        assert_eq!(wrapped.program, MACOS_SANDBOX_EXEC);
        assert!(wrapped.sandboxed);
        assert_eq!(wrapped.sandbox_backend, "macos-seatbelt");
        assert_eq!(
            wrapped.sandbox_home.as_deref(),
            Some(manager.sandbox_home.as_path())
        );

        let cleanup_path = wrapped
            .cleanup_path
            .expect("macos wrapper should return cleanup path");

        assert!(cleanup_path.exists(), "profile should be created on disk");
        assert!(
            wrapped
                .args
                .iter()
                .any(|arg| arg == &cleanup_path.display().to_string()),
            "wrapped command should reference the generated profile path"
        );

        let profile = std::fs::read_to_string(&cleanup_path).expect("profile contents");
        assert!(
            !profile.contains("home-relative-path"),
            "profile should avoid unsupported home-relative-path forms"
        );
        assert!(
            profile.contains("(deny file-read* (subpath "),
            "profile should deny sensitive home subpaths using explicit paths"
        );
    }

    #[test]
    fn wrap_command_can_be_explicitly_direct() {
        let manager = manager("macos", SandboxBackend::Direct);

        let wrapped = manager
            .wrap_shell_command("find . -maxdepth 1", "/workspace/project", false)
            .expect("direct fallback wrapper");

        assert_eq!(wrapped.program, shell_program());
        assert!(matches!(
            wrapped.args.as_slice(),
            [flag, command] if (flag == "-c" || flag == "-lc") && command == "find . -maxdepth 1"
        ));
        assert!(wrapped.cleanup_path.is_none());
        assert!(
            !wrapped.sandboxed,
            "direct execution should not apply sandbox env or profiles"
        );
        assert_eq!(wrapped.sandbox_backend, "direct");
        assert!(wrapped.sandbox_home.is_none());
    }

    #[test]
    fn wrap_pty_shell_command_uses_direct_backend_on_macos() {
        let manager = manager("macos", SandboxBackend::MacosSeatbelt);

        let wrapped = manager
            .wrap_pty_shell_command("read line", "/workspace/project", false)
            .expect("pty shell wrapper");

        assert!(!wrapped.sandboxed);
        assert_eq!(wrapped.sandbox_backend, "direct");
    }

    #[test]
    fn shell_command_flag_uses_login_shell_for_common_user_shells() {
        assert_eq!(shell_command_flag("/bin/zsh"), "-lc");
        assert_eq!(shell_command_flag("/usr/bin/bash"), "-lc");
        assert_eq!(shell_command_flag("sh"), "-c");
    }

    #[test]
    fn sanitize_shell_program_rejects_args_and_unmounted_shells() {
        assert_eq!(
            sanitize_shell_program("/bin/zsh"),
            Some("/bin/zsh".to_string())
        );
        assert_eq!(
            sanitize_shell_program("/bin/zsh -i"),
            Some("/bin/zsh".to_string())
        );
        assert_eq!(sanitize_shell_program("/opt/homebrew/bin/fish"), None);
        assert_eq!(sanitize_shell_program("zsh"), None);
        assert_eq!(DEFAULT_SHELL, "/bin/sh");
    }

    #[test]
    fn new_keeps_recent_macos_profiles() {
        let tempdir = tempdir().expect("tempdir");
        let rara_dir = tempdir.path().join(".rara");
        let profile_dir = rara_dir.join("sandbox");
        std::fs::create_dir_all(&profile_dir).expect("profile dir");
        let recent_profile = profile_dir.join("sandbox-recent.sb");
        std::fs::write(&recent_profile, "(version 1)").expect("recent profile");

        let manager = SandboxManager::new_for_rara_dir(rara_dir.clone()).expect("sandbox manager");

        assert!(
            manager.profile_dir == rara_dir.join("sandbox"),
            "sandbox manager should point at the configured sandbox dir"
        );
        assert!(
            recent_profile.exists(),
            "recent sandbox profiles should not be removed on startup"
        );
    }

    #[test]
    fn cleanup_removes_profiles_when_they_are_older_than_the_threshold() {
        let tempdir = tempdir().expect("tempdir");
        let stale_profile = tempdir.path().join("sandbox-stale.sb");
        let unrelated_file = tempdir.path().join("notes.txt");
        std::fs::write(&stale_profile, "(version 1)").expect("stale profile");
        std::fs::write(&unrelated_file, "keep").expect("unrelated file");

        cleanup_profiles_older_than(tempdir.path(), Duration::ZERO).expect("cleanup");

        assert!(
            !stale_profile.exists(),
            "matching profiles should be removed when past the cleanup threshold"
        );
        assert!(
            unrelated_file.exists(),
            "non-profile files must not be removed by sandbox cleanup"
        );
    }

    #[test]
    fn cleanup_keeps_recent_profiles_with_default_threshold() {
        let tempdir = tempdir().expect("tempdir");
        let recent_profile = tempdir.path().join("sandbox-recent.sb");
        std::fs::write(&recent_profile, "(version 1)").expect("recent profile");

        cleanup_stale_profiles(tempdir.path()).expect("cleanup");

        assert!(
            recent_profile.exists(),
            "startup cleanup should not remove profiles that may belong to active instances"
        );
    }

    #[test]
    fn process_sandbox_home_uses_tmp_root_and_unique_names() {
        let first = super::process_sandbox_home();
        let second = super::process_sandbox_home();

        assert!(first.starts_with("/tmp"));
        assert!(second.starts_with("/tmp"));
        assert_ne!(
            first, second,
            "sandbox home paths must not collide between manager instances"
        );
    }

    #[test]
    fn wrap_shell_command_uses_minimal_linux_bind_set() {
        let manager = manager("linux", SandboxBackend::LinuxBubblewrap);

        let wrapped = manager
            .wrap_shell_command("echo test", "/workspace/project", false)
            .expect("linux sandbox wrapper");

        assert_eq!(wrapped.program, "bwrap");
        assert!(
            !wrapped.args.windows(3).any(|window| window
                == [
                    String::from("--ro-bind"),
                    String::from("/"),
                    String::from("/")
                ]),
            "linux sandbox should no longer bind the entire filesystem read-only"
        );
        assert!(
            wrapped
                .args
                .windows(2)
                .any(|window| window == [String::from("--tmpfs"), String::from("/")]),
            "linux sandbox should start from an empty root filesystem"
        );
        assert!(
            wrapped
                .args
                .windows(2)
                .any(|window| window == [String::from("--tmpfs"), String::from("/tmp")]),
            "linux sandbox should provide an isolated writable /tmp"
        );
        assert!(
            wrapped.args.windows(2).any(
                |window| window == [String::from("--bind"), String::from("/workspace/project")]
            ),
            "linux sandbox should bind the workspace path back in"
        );
        assert!(
            wrapped.args.contains(&"--unshare-net".to_string()),
            "linux sandbox should isolate networking when allow_net is false"
        );
        assert_eq!(wrapped.sandbox_backend, "linux-bubblewrap");
        assert_eq!(
            wrapped.sandbox_home.as_deref(),
            Some(manager.sandbox_home.as_path())
        );
    }

    #[test]
    fn linux_sandbox_creates_home_dirs_inside_bubblewrap() {
        let manager = manager("linux", SandboxBackend::LinuxBubblewrap);

        let wrapped = manager
            .wrap_shell_command("echo test", "/workspace/project", false)
            .expect("linux sandbox wrapper");

        for dir in [
            manager.sandbox_home.clone(),
            manager.sandbox_home.join(".config"),
            manager.sandbox_home.join(".cache"),
            manager.sandbox_home.join(".local/state"),
            manager.sandbox_home.join(".local/share"),
        ] {
            assert!(
                wrapped
                    .args
                    .windows(2)
                    .any(|window| { window == [String::from("--dir"), dir.display().to_string()] }),
                "linux sandbox should create {} inside the tmpfs root",
                dir.display()
            );
        }
    }

    #[test]
    fn linux_sandbox_does_not_bind_the_entire_home_directory() {
        let manager = manager("linux", SandboxBackend::LinuxBubblewrap);

        let wrapped = manager
            .wrap_shell_command("echo test", "/home/tester/work/project", false)
            .expect("linux sandbox wrapper");

        assert_eq!(wrapped.program, "bwrap");
        assert!(
            !wrapped.args.windows(3).any(|window| {
                window
                    == [
                        String::from("--bind"),
                        String::from("/home/tester"),
                        String::from("/home/tester"),
                    ]
                    || window
                        == [
                            String::from("--ro-bind"),
                            String::from("/home/tester"),
                            String::from("/home/tester"),
                        ]
            }),
            "linux sandbox should not mount the entire home directory back in"
        );
        assert!(
            wrapped.args.windows(3).any(|window| {
                window
                    == [
                        String::from("--bind"),
                        String::from("/home/tester/work/project"),
                        String::from("/home/tester/work/project"),
                    ]
            }),
            "linux sandbox should still mount the workspace path itself"
        );
    }

    #[test]
    fn linux_sandbox_binds_command_search_path_dirs() {
        let tempdir = tempdir().expect("tempdir");
        let custom_bin = tempdir.path().join("custom-bin");
        std::fs::create_dir_all(&custom_bin).expect("custom bin dir");
        let custom_bin = std::fs::canonicalize(&custom_bin).expect("canonical custom bin");
        let original_path = std::env::var_os("PATH");
        let test_path =
            env::join_paths([PathBuf::from("."), custom_bin.clone()]).expect("build test PATH");
        set_env_var("PATH", test_path);

        let manager = manager("linux", SandboxBackend::LinuxBubblewrap);
        let wrapped = manager
            .wrap_shell_command("custom-tool --version", "/workspace/project", false)
            .expect("linux sandbox wrapper");

        if let Some(path) = original_path {
            set_env_var("PATH", path);
        } else {
            remove_env_var("PATH");
        }

        assert!(
            wrapped.args.windows(3).any(|window| {
                window
                    == [
                        String::from("--ro-bind"),
                        custom_bin.display().to_string(),
                        custom_bin.display().to_string(),
                    ]
            }),
            "linux sandbox should bind PATH command directories that are outside standard runtime roots"
        );
        assert!(
            !wrapped.args.windows(3).any(|window| {
                window
                    == [
                        String::from("--ro-bind"),
                        String::from("."),
                        String::from("."),
                    ]
            }),
            "linux sandbox should not bind relative PATH entries"
        );
    }

    #[test]
    fn linux_sandbox_binds_command_install_roots_for_bin_dirs() {
        let tempdir = tempdir().expect("tempdir");
        let tool_root = tempdir.path().join("toolchain");
        let tool_bin = tool_root.join("bin");
        std::fs::create_dir_all(&tool_bin).expect("tool bin dir");
        let tool_root = std::fs::canonicalize(&tool_root).expect("canonical tool root");
        let tool_bin = tool_root.join("bin");
        let original_path = std::env::var_os("PATH");
        set_env_var("PATH", &tool_bin);

        let manager = manager("linux", SandboxBackend::LinuxBubblewrap);
        let wrapped = manager
            .wrap_shell_command("custom-tool --version", "/workspace/project", false)
            .expect("linux sandbox wrapper");

        if let Some(path) = original_path {
            set_env_var("PATH", path);
        } else {
            remove_env_var("PATH");
        }

        assert!(
            wrapped.args.windows(3).any(|window| {
                window
                    == [
                        String::from("--ro-bind"),
                        tool_root.display().to_string(),
                        tool_root.display().to_string(),
                    ]
            }),
            "linux sandbox should bind the PATH install root so symlinked launchers and adjacent libraries remain readable"
        );
    }

    #[test]
    fn macos_profile_allows_command_install_roots() {
        let tempdir = tempdir().expect("tempdir");
        let tool_root = tempdir.path().join("toolchain");
        let tool_bin = tool_root.join("bin");
        std::fs::create_dir_all(&tool_bin).expect("tool bin dir");
        let tool_root = std::fs::canonicalize(&tool_root).expect("canonical tool root");
        let original_path = std::env::var_os("PATH");
        set_env_var("PATH", tool_root.join("bin"));
        let manager = SandboxManager {
            os: "macos".to_string(),
            profile_dir: tempdir.path().to_path_buf(),
            sandbox_home: tempdir.path().join("home"),
            backend: SandboxBackend::MacosSeatbelt,
            command_install_roots: command_search_install_roots(Some(
                tool_root.join("bin").as_os_str(),
            )),
        };

        let profile_path = manager.create_profile(false).expect("macos profile");

        if let Some(path) = original_path {
            set_env_var("PATH", path);
        } else {
            remove_env_var("PATH");
        }

        let profile = std::fs::read_to_string(profile_path).expect("profile contents");
        let expected = tool_root.display().to_string();
        assert!(
            profile.contains(&format!("(allow file-read* (subpath \"{expected}\"))")),
            "macos profile should allow reading PATH install roots"
        );
        assert!(
            profile.contains(&format!(
                "(allow file-map-executable (subpath \"{expected}\"))"
            )),
            "macos profile should allow mapping executables from PATH install roots"
        );
    }

    #[test]
    fn home_bin_path_stays_narrow() {
        let tempdir = tempdir().expect("tempdir");
        let home = tempdir.path().join("home");
        let home_bin = home.join("bin");
        std::fs::create_dir_all(&home_bin).expect("home bin dir");
        let home = std::fs::canonicalize(&home).expect("canonical home");
        let home_bin = home.join("bin");
        let original_home = std::env::var_os("HOME");
        set_env_var("HOME", &home);

        let roots = command_search_install_roots(Some(home_bin.as_os_str()));

        if let Some(home) = original_home {
            set_env_var("HOME", home);
        } else {
            remove_env_var("HOME");
        }

        assert_eq!(roots, vec![home_bin]);
    }

    #[test]
    fn sandbox_profile_string_literal_escapes_control_characters() {
        let escaped = sandbox_profile_string_literal(PathBuf::from("/tmp/a\nb\tc").as_path());

        assert_eq!(escaped, "/tmp/a\\nb\\tc");
    }
}
