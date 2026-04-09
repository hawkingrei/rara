use serde::{Deserialize, Serialize};
use anyhow::{Result, anyhow};
use std::path::PathBuf;
use rand::{distributions::Alphanumeric, Rng};
use sha2::{Digest, Sha256};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use tokio::net::TcpListener;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e"; 
const TOKEN_URL: &str = "https://platform.ai-gateway.com/v1/oauth/token";

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OAuthToken {
    pub access_token: String,
    pub refresh_token: Option<String>,
}

pub struct OAuthManager { pub config_dir: PathBuf }
impl OAuthManager {
    pub fn new() -> Result<Self> { 
        let d = std::env::current_dir()?.join(".rara");
        if !d.exists() { std::fs::create_dir_all(&d)?; }
        Ok(Self { config_dir: d }) 
    }

    pub fn generate_pkce(&self) -> (String, String) {
        let v: String = rand::thread_rng().sample_iter(&Alphanumeric).take(128).map(char::from).collect();
        let mut h = Sha256::new(); h.update(v.as_bytes());
        (v, URL_SAFE_NO_PAD.encode(h.finalize()))
    }

    pub async fn start_callback_server(&self) -> Result<(u16, tokio::sync::oneshot::Receiver<String>)> {
        let l = TcpListener::bind("127.0.0.1:0").await?;
        let p = l.local_addr()?.port();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            if let Ok((mut s, _)) = l.accept().await {
                let mut r = BufReader::new(&mut s);
                let mut line = String::new();
                if r.read_line(&mut line).await.is_ok() {
                    if let Some(c) = line.split_whitespace().nth(1).and_then(|p| p.split("code=").nth(1)).and_then(|c| c.split('&').next()) {
                        let _ = s.write_all(b"HTTP/1.1 200 OK\r\n\r\nLogin Success").await;
                        let _ = tx.send(c.to_string());
                    }
                }
            }
        });
        Ok((p, rx))
    }

    pub async fn exchange_code(&self, code: &str, verifier: &str, port: u16) -> Result<OAuthToken> {
        let params = [
            ("grant_type", "authorization_code"), ("code", code), ("client_id", CLIENT_ID),
            ("code_verifier", verifier), ("redirect_uri", &format!("http://localhost:{}/callback", port)),
        ];
        let res = reqwest::Client::new().post(TOKEN_URL).form(&params).send().await?;
        if !res.status().is_success() { return Err(anyhow!("OAuth Fail")); }
        Ok(res.json().await?)
    }

    pub fn get_authorize_url(&self, challenge: &str, port: u16) -> String {
        format!("https://ai-gateway.com/cai/oauth/authorize?response_type=code&client_id={}&code_challenge={}&code_challenge_method=S256&redirect_uri={}",
            CLIENT_ID, challenge, urlencoding::encode(&format!("http://localhost:{}/callback", port)))
    }
}
