use std::process::Command;
use std::env;
use anyhow::{Result, anyhow};
use std::fs;
use std::path::PathBuf;
use std::collections::HashSet;

pub struct SandboxManager {
    os: String,
    profile_path: PathBuf,
}

impl SandboxManager {
    pub fn new() -> Result<Self> {
        let os = std::env::consts::OS.to_string();
        let rara_dir = std::env::current_dir()?.join(".rara");
        if !rara_dir.exists() { fs::create_dir_all(&rara_dir)?; }
        let profile_path = rara_dir.join("sandbox.sb");

        let manager = Self { os, profile_path };
        manager.update_profile(false)?; 
        Ok(manager)
    }

    fn get_proxy_hosts(&self) -> Vec<String> {
        let mut proxies = HashSet::new();
        for var in &["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"] {
            if let Ok(url_str) = env::var(var) {
                if let Ok(url) = url::Url::parse(&url_str) {
                    if let Some(host) = url.host_str() { proxies.insert(host.to_string()); }
                }
            }
        }
        proxies.into_iter().collect()
    }

    pub fn update_profile(&self, allow_net: bool) -> Result<()> {
        if self.os != "macos" { return Ok(()); }
        let mut net_rules = String::new();
        if allow_net { net_rules.push_str("(allow network*)"); }
        else {
            net_rules.push_str("(deny network*)\n(allow network-outbound (literal \"/private/var/run/mDNSResponder\"))\n");
            for host in self.get_proxy_hosts() { net_rules.push_str(&format!("(allow network-outbound (remote ip \"{}:*\"))\n", host)); }
        }

        let profile = format!(r#"(version 1)
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
"#, net_rules);
        fs::write(&self.profile_path, profile)?;
        Ok(())
    }

    pub fn wrap_command(&self, original_cmd: &str, cwd: &str, allow_net: bool) -> Result<String> {
        self.update_profile(allow_net)?;
        match self.os.as_str() {
            "macos" => Ok(format!("sandbox-exec -D CWD=\"{}\" -f \"{}\" sh -c \"{}\"", cwd, self.profile_path.display(), original_cmd.replace("\"", "\\\""))),
            "linux" => {
                let net = if allow_net { "" } else { "--unshare-net" };
                Ok(format!("bwrap --ro-bind / / --dev /dev --proc /proc --bind \"{cwd}\" \"{cwd}\" {net} sh -c \"{}\"", original_cmd.replace("\"", "\\\""), cwd=cwd, net=net))
            },
            _ => Ok(original_cmd.to_string()),
        }
    }

    pub fn explain_violation(&self, stderr: &str) -> Option<String> {
        if stderr.contains("Operation not permitted") || stderr.contains("Sandbox: Violation") { Some("Blocked by RARA Sandbox.".into()) } else { None }
    }
}
