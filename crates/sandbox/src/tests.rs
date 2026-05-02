use super::{
    SandboxBackend, SandboxManager, DEFAULT_SHELL, MACOS_SANDBOX_EXEC,
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
        command_install_roots: command_search_install_roots(std::env::var_os("PATH").as_deref()),
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
        command_install_roots: command_search_install_roots(std::env::var_os("PATH").as_deref()),
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
        cleanup_path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("sandbox-") && name.ends_with(".sb")),
        "profile filename should follow the sandbox-*.sb pattern"
    );
}

#[test]
fn direct_exec_wrapper_preserves_program_and_args() {
    let manager = manager("macos", SandboxBackend::Direct);

    let wrapped = manager.wrap_unsandboxed_exec_command("echo", &["hello".to_string()]);

    assert!(!wrapped.sandboxed);
    assert_eq!(wrapped.program, "echo");
    assert_eq!(wrapped.args, vec!["hello".to_string()]);
    assert_eq!(wrapped.cleanup_path, None);
}

#[test]
fn wrap_pty_shell_command_uses_direct_mode_on_macos() {
    let manager = manager("macos", SandboxBackend::MacosSeatbelt);

    let wrapped = manager
        .wrap_pty_shell_command("custom-tool --flag", "/tmp", false)
        .expect("pty shell wrapper");

    assert_eq!(wrapped.program, "/bin/sh");
    assert_eq!(wrapped.args, vec!["-c".to_string(), "custom-tool --flag".to_string()]);
    assert!(!wrapped.sandboxed);
}

#[test]
fn wrap_shell_command_uses_platform_shell() {
    let manager = manager("macos", SandboxBackend::MacosSeatbelt);

    let wrapped = manager
        .wrap_shell_command("custom-tool --flag", "/tmp", false)
        .expect("macos shell wrapper");

    assert_eq!(wrapped.program, MACOS_SANDBOX_EXEC);
    let args_str = wrapped.args.join(" ");
    assert!(
        args_str.contains("/bin/zsh") || args_str.contains("/bin/bash") || args_str.contains(DEFAULT_SHELL),
        "macos shell wrapper should use a known system shell, got args: {args_str}"
    );
}

#[test]
fn sandbox_profile_string_literal_escapes_backslashes_and_quotes() {
    let path = PathBuf::from(r#"/tmp/test"dir"#);
    let literal = sandbox_profile_string_literal(&path);
    assert_eq!(literal, r#"/tmp/test\"dir"#);
}

#[test]
fn sanitize_shell_program_returns_none_for_relative_or_unknown_paths() {
    assert_eq!(
        sanitize_shell_program("/bin/zsh"),
        Some("/bin/zsh".to_string())
    );
    assert_eq!(
        sanitize_shell_program("/bin/bash"),
        Some("/bin/bash".to_string())
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
        wrapped
            .args
            .windows(3)
            .any(|window| window
                == [
                    String::from("--ro-bind"),
                    String::from("/workspace/project"),
                    String::from("/workspace/project")
                ]),
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
        command_install_roots: command_search_install_roots(std::env::var_os("PATH").as_deref()),
    };

    let wrapped = manager
        .wrap_shell_command("echo test", "/tmp", false)
        .expect("macos sandbox wrapper");

    if let Some(path) = original_path {
        set_env_var("PATH", path);
    } else {
        remove_env_var("PATH");
    }

    let profile = std::fs::read_to_string(wrapped.cleanup_path.as_ref().expect("cleanup path"))
        .expect("read profile");
    assert!(
        profile.contains(&format!(
            "(allow file-read* (subpath \"{}\"))",
            sandbox_profile_string_literal(&tool_root)
        )),
        "macOS sandbox profile should emit a file-read* subpath rule for tool install roots"
    );
    assert!(
        profile.contains(&format!(
            "(allow file-map-executable (subpath \"{}\"))",
            sandbox_profile_string_literal(&tool_root)
        )),
        "macOS sandbox profile should emit a file-map-executable subpath rule for tool install roots"
    );
}

#[test]
fn shell_command_flag_includes_full_shell_and_source_path() {
    let flag = shell_command_flag(
        "/bin/zsh",
        "/tmp",
        "/tmp/rara-sandbox-home",
        false,
        Default::default(),
    );
    assert!(flag.contains("ZSH=/bin/zsh"));
    assert!(flag.contains("source /tmp"));
    assert!(flag.contains("HOME=/tmp/rara-sandbox-home"));
    assert!(!flag.contains("NETWORK_ACCESS=1"));
}

#[test]
fn shell_command_flag_enables_network_access_when_allow_net() {
    let flag = shell_command_flag(
        "/bin/bash",
        "/tmp",
        "/tmp/rara-sandbox-home",
        true,
        Default::default(),
    );
    assert!(flag.contains("NETWORK_ACCESS=1"));
}
