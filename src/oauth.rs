use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::{distributions::Alphanumeric, Rng};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const ISSUER: &str = "https://auth.openai.com";
const DEVICE_AUTH_MAX_WAIT_SECS: u64 = 15 * 60;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OAuthToken {
    #[serde(
        serialize_with = "serialize_secret",
        deserialize_with = "deserialize_secret"
    )]
    pub access_token: SecretString,
    #[serde(
        default,
        serialize_with = "serialize_secret_option",
        deserialize_with = "deserialize_secret_option"
    )]
    pub refresh_token: Option<SecretString>,
    #[serde(
        default,
        serialize_with = "serialize_secret_option",
        deserialize_with = "deserialize_secret_option"
    )]
    pub id_token: Option<SecretString>,
}

#[derive(Debug, Clone)]
pub struct DeviceCode {
    pub verification_url: String,
    pub user_code: String,
    device_auth_id: String,
    interval_secs: u64,
}

fn serialize_secret<S>(value: &SecretString, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(value.expose_secret())
}

fn deserialize_secret<'de, D>(deserializer: D) -> Result<SecretString, D::Error>
where
    D: serde::Deserializer<'de>,
{
    String::deserialize(deserializer).map(SecretString::from)
}

fn serialize_secret_option<S>(
    value: &Option<SecretString>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    Option::<String>::serialize(
        &value.as_ref().map(|secret| secret.expose_secret().to_string()),
        serializer,
    )
}

fn deserialize_secret_option<'de, D>(deserializer: D) -> Result<Option<SecretString>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    Ok(value.map(SecretString::from))
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

    pub fn codex_issuer(&self) -> &'static str {
        ISSUER
    }

    pub fn client_id(&self) -> &'static str {
        CLIENT_ID
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
            ("grant_type", "authorization_code"),
            ("code", code),
            ("client_id", CLIENT_ID),
            ("code_verifier", verifier),
            ("redirect_uri", &format!("http://localhost:{}/callback", port)),
        ];
        let token_url = format!("{ISSUER}/oauth/token");
        let res = reqwest::Client::new()
            .post(token_url)
            .form(&params)
            .send()
            .await?;
        if !res.status().is_success() {
            return Err(anyhow!("OAuth token exchange failed with status {}", res.status()));
        }
        Ok(res.json().await?)
    }

    pub fn get_authorize_url(&self, challenge: &str, port: u16) -> String {
        format!(
            "{ISSUER}/oauth/authorize?response_type=code&client_id={}&code_challenge={}&code_challenge_method=S256&redirect_uri={}",
            CLIENT_ID,
            challenge,
            urlencoding::encode(&format!("http://localhost:{}/callback", port))
        )
    }

    pub async fn request_device_code(&self) -> Result<DeviceCode> {
        #[derive(Serialize)]
        struct UserCodeReq<'a> {
            client_id: &'a str,
        }

        #[derive(Deserialize)]
        struct UserCodeResp {
            device_auth_id: String,
            #[serde(alias = "user_code", alias = "usercode")]
            user_code: String,
            #[serde(default, deserialize_with = "deserialize_interval_secs")]
            interval: u64,
        }

        let url = format!("{ISSUER}/api/accounts/deviceauth/usercode");
        let body = serde_json::to_string(&UserCodeReq { client_id: CLIENT_ID })?;
        let response = reqwest::Client::new()
            .post(url)
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(anyhow!(
                "device code request failed with status {}",
                response.status()
            ));
        }

        let payload: UserCodeResp = response.json().await?;
        Ok(DeviceCode {
            verification_url: format!("{ISSUER}/codex/device"),
            user_code: payload.user_code,
            device_auth_id: payload.device_auth_id,
            interval_secs: payload.interval.max(1),
        })
    }

    pub async fn complete_device_code_login(&self, device_code: &DeviceCode) -> Result<OAuthToken> {
        #[derive(Serialize)]
        struct TokenPollReq<'a> {
            device_auth_id: &'a str,
            user_code: &'a str,
        }

        #[derive(Deserialize)]
        struct CodeSuccessResp {
            authorization_code: String,
            code_verifier: String,
        }

        let poll_url = format!("{ISSUER}/api/accounts/deviceauth/token");
        let started = std::time::Instant::now();
        let response = loop {
            if started.elapsed().as_secs() >= DEVICE_AUTH_MAX_WAIT_SECS {
                return Err(anyhow!("device code login timed out after 15 minutes"));
            }

            let body = serde_json::to_string(&TokenPollReq {
                device_auth_id: &device_code.device_auth_id,
                user_code: &device_code.user_code,
            })?;
            let response = reqwest::Client::new()
                .post(&poll_url)
                .header("Content-Type", "application/json")
                .body(body)
                .send()
                .await?;

            if response.status().is_success() {
                let code_response: CodeSuccessResp = response.json().await?;
                break code_response;
            }

            if response.status() == reqwest::StatusCode::FORBIDDEN
                || response.status() == reqwest::StatusCode::NOT_FOUND
            {
                tokio::time::sleep(std::time::Duration::from_secs(device_code.interval_secs)).await;
                continue;
            }

            return Err(anyhow!(
                "device code poll failed with status {}",
                response.status()
            ));
        };

        let redirect_uri = format!("{ISSUER}/deviceauth/callback");
        let params = [
            ("grant_type", "authorization_code"),
            ("code", response.authorization_code.as_str()),
            ("client_id", CLIENT_ID),
            ("code_verifier", response.code_verifier.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
        ];
        let token_url = format!("{ISSUER}/oauth/token");
        let result = reqwest::Client::new()
            .post(token_url)
            .form(&params)
            .send()
            .await?;
        if !result.status().is_success() {
            return Err(anyhow!(
                "device code exchange failed with status {}",
                result.status()
            ));
        }
        Ok(result.json().await?)
    }

    pub fn read_api_key_from_stdin(&self) -> Result<SecretString> {
        use std::io::IsTerminal;

        let mut stdin = std::io::stdin();
        if stdin.is_terminal() {
            return Err(anyhow!(
                "--with-api-key expects the API key on stdin"
            ));
        }

        let mut buffer = String::new();
        stdin.read_to_string(&mut buffer)?;
        let api_key = buffer.trim();
        if api_key.is_empty() {
            return Err(anyhow!("no API key provided via stdin"));
        }
        Ok(SecretString::from(api_key.to_string()))
    }
}

fn deserialize_interval_secs<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum IntervalValue {
        Number(u64),
        String(String),
    }

    match IntervalValue::deserialize(deserializer)? {
        IntervalValue::Number(value) => Ok(value),
        IntervalValue::String(value) => value
            .trim()
            .parse::<u64>()
            .map_err(serde::de::Error::custom),
    }
}

#[cfg(test)]
mod tests {
    use super::OAuthManager;
    use tempfile::tempdir;

    #[test]
    fn authorize_url_uses_codex_issuer() {
        let temp = tempdir().expect("tempdir");
        let manager = OAuthManager {
            config_dir: temp.path().join(".rara"),
        };
        let url = manager.get_authorize_url("challenge", 1455);
        assert!(url.starts_with("https://auth.openai.com/oauth/authorize"));
        assert!(url.contains("client_id=app_EMoamEEZ73f0CkXaXp7hrann"));
    }
}
