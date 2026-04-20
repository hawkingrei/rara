use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::{bail, Result};
use uuid::Uuid;

pub struct SandboxManager {
    os: String,
    profile_dir: PathBuf,
}

#[derive(Debug)]
pub struct WrappedCommand {
    pub program: String,
    pub args: Vec<String>,
    pub cleanup_path: Option<PathBuf>,
}

const LINUX_RUNTIME_READ_ROOTS: &[&str] = &[
    "/bin",
    "/sbin",
    "/usr",
    "/etc",
    "/lib",
    "/lib64",
    "/nix/store",
    "/run/current-system/sw",
];

impl SandboxManager {
    pub fn new() -> Result<Self> {
        let os = std::env::consts::OS.to_string();
        let rara_dir = std::env::current_dir()?.join(".rara");
        if !rara_dir.exists() {
            fs::create_dir_all(&rara_dir)?;
        }
        let profile_dir = rara_dir.join("sandbox");
        if !profile_dir.exists() {
            fs::create_dir_all(&profile_dir)?;
        }
        cleanup_stale_profiles(&profile_dir)?;

        Ok(Self { os, profile_dir })
    }

    fn get_proxy_hosts(&self) -> Vec<String> {
        let mut proxies = HashSet::new();
        for var in &["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"] {
            if let Ok(url_str) = env::var(var) {
                if let Ok(url) = url::Url::parse(&url_str) {
                    if let Some(host) = url.host_str() {
                        proxies.insert(host.to_string());
                    }
                }
            }
        }
        proxies.into_iter().collect()
    }

    fn create_profile(&self, allow_net: bool) -> Result<PathBuf> {
        if self.os != "macos" {
            bail!("sandbox profile creation is only supported on macOS");
        }

        let mut net_rules = String::new();
        if allow_net {
            net_rules.push_str("(allow network*)");
        } else {
            net_rules.push_str("(deny network*)\n(allow network-outbound (literal \"/private/var/run/mDNSResponder\"))\n");
            for host in self.get_proxy_hosts() {
                net_rules.push_str(&format!(
                    "(allow network-outbound (remote ip \"{}:*\"))\n",
                    host
                ));
            }
        }

        let profile = format!(
            r#"(version 1)
(deny default)
(deny file-read* (home-relative-path "/.ssh"))
(deny file-read* (home-relative-path "/.aws"))
(allow file-read* (subpath "/usr/bin"))
(allow file-read* (subpath "/System"))
(allow file-read* (subpath (param "CWD")))
(allow file-write* (subpath (param "CWD")))
(allow file-write* (subpath "/tmp"))
(allow process*)
(allow sysctl-read)
{}
"#,
            net_rules
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
        ];

        for root in LINUX_RUNTIME_READ_ROOTS {
            if Path::new(root).exists() {
                args.push("--ro-bind".to_string());
                args.push((*root).to_string());
                args.push((*root).to_string());
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
        match self.os.as_str() {
            "macos" => {
                let profile_path = self.create_profile(allow_net)?;
                Ok(WrappedCommand {
                    program: "sandbox-exec".to_string(),
                    args: vec![
                        "-D".to_string(),
                        format!("CWD={cwd}"),
                        "-f".to_string(),
                        profile_path.display().to_string(),
                        "sh".to_string(),
                        "-c".to_string(),
                        original_cmd.to_string(),
                    ],
                    cleanup_path: Some(profile_path),
                })
            }
            "linux" => {
                let mut args = self.linux_sandbox_args(cwd, allow_net);
                args.push("--".to_string());
                args.push("sh".to_string());
                args.push("-c".to_string());
                args.push(original_cmd.to_string());
                Ok(WrappedCommand {
                    program: "bwrap".to_string(),
                    args,
                    cleanup_path: None,
                })
            }
            _ => bail!(
                "sandboxed command execution is unsupported on platform {}",
                self.os
            ),
        }
    }

    pub fn wrap_exec_command(
        &self,
        program: &str,
        args: &[String],
        cwd: &str,
        allow_net: bool,
    ) -> Result<WrappedCommand> {
        match self.os.as_str() {
            "macos" => {
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
                    program: "sandbox-exec".to_string(),
                    args: wrapped_args,
                    cleanup_path: Some(profile_path),
                })
            }
            "linux" => {
                let mut wrapped_args = self.linux_sandbox_args(cwd, allow_net);
                wrapped_args.push("--".to_string());
                wrapped_args.push(program.to_string());
                wrapped_args.extend(args.iter().cloned());
                Ok(WrappedCommand {
                    program: "bwrap".to_string(),
                    args: wrapped_args,
                    cleanup_path: None,
                })
            }
            _ => bail!(
                "sandboxed command execution is unsupported on platform {}",
                self.os
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

fn cleanup_stale_profiles(profile_dir: &Path) -> Result<()> {
    for entry in fs::read_dir(profile_dir)? {
        let entry = entry?;
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if path.is_file() && file_name.starts_with("sandbox-") && file_name.ends_with(".sb") {
            let _ = fs::remove_file(&path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::SandboxManager;
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;

    #[test]
    fn wrap_command_fails_closed_on_unsupported_platform() {
        let manager = SandboxManager {
            os: "freebsd".to_string(),
            profile_dir: PathBuf::from("/tmp/rara-test-sandbox"),
        };

        let err = manager
            .wrap_shell_command("echo test", "/tmp", false)
            .expect_err("unsupported platforms should not fall back to unsandboxed execution");

        assert!(err
            .to_string()
            .contains("sandboxed command execution is unsupported on platform freebsd"));
    }

    #[test]
    fn wrap_command_creates_unique_cleanup_profile_on_macos() {
        let tempdir = tempdir().expect("tempdir");
        let manager = SandboxManager {
            os: "macos".to_string(),
            profile_dir: tempdir.path().to_path_buf(),
        };

        let wrapped = manager
            .wrap_shell_command("echo test", "/tmp", false)
            .expect("macos sandbox wrapper");

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
    }

    #[test]
    fn new_removes_stale_macos_profiles() {
        let tempdir = tempdir().expect("tempdir");
        let rara_dir = tempdir.path().join(".rara");
        let profile_dir = rara_dir.join("sandbox");
        std::fs::create_dir_all(&profile_dir).expect("profile dir");
        let stale_profile = profile_dir.join("sandbox-stale.sb");
        std::fs::write(&stale_profile, "(version 1)").expect("stale profile");

        let current_dir = std::env::current_dir().expect("cwd");
        std::env::set_current_dir(tempdir.path()).expect("switch cwd");
        let manager = SandboxManager::new().expect("sandbox manager");
        std::env::set_current_dir(current_dir).expect("restore cwd");

        assert!(
            manager
                .profile_dir
                .ends_with(Path::new(".rara").join("sandbox")),
            "sandbox manager should point at the workspace-local sandbox dir"
        );
        assert!(
            !stale_profile.exists(),
            "stale sandbox profiles should be cleaned up on startup"
        );
    }

    #[test]
    fn wrap_shell_command_uses_minimal_linux_bind_set() {
        let manager = SandboxManager {
            os: "linux".to_string(),
            profile_dir: PathBuf::from("/tmp/rara-test-sandbox"),
        };

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
    }
}
