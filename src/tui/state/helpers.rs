use std::process::Command;

fn command_stdout(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub(super) fn detect_current_pr_url() -> Option<String> {
    command_stdout("gh", &["pr", "view", "--json", "url", "-q", ".url"])
}

pub(super) fn detect_repo_slug() -> Option<String> {
    let remote = command_stdout("git", &["remote", "get-url", "origin"])?;
    parse_repo_slug(remote.as_str())
}

pub(super) fn detect_repo_context() -> (Option<String>, Option<String>) {
    (detect_repo_slug(), detect_current_pr_url())
}

pub(crate) fn parse_repo_slug(remote: &str) -> Option<String> {
    let remote = remote.trim();
    if remote.is_empty() {
        return None;
    }

    let stripped = remote
        .strip_prefix("git@github.com:")
        .or_else(|| remote.strip_prefix("https://github.com/"))
        .or_else(|| remote.strip_prefix("ssh://git@github.com/"))?;
    Some(stripped.trim_end_matches(".git").to_string())
}
