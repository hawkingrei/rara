use anyhow::{anyhow, Result};
use codex_login::{
    complete_device_code_login as codex_complete_device_code_login, load_auth_dot_json,
    login_with_api_key as codex_login_with_api_key, logout as codex_logout,
    request_device_code as codex_request_device_code, run_login_server as codex_run_login_server,
    AuthCredentialsStoreMode, DeviceCode as CodexDeviceCode, LoginServer as CodexLoginServer,
    ServerOptions, CLIENT_ID,
};
use secrecy::SecretString;
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const ISSUER: &str = "https://auth.openai.com";

#[derive(Debug, Clone)]
pub struct DeviceCode {
    pub verification_url: String,
    pub user_code: String,
    inner: CodexDeviceCode,
}

pub struct BrowserLoginSession {
    auth_url: String,
    inner: CodexLoginServer,
}

impl BrowserLoginSession {
    pub fn auth_url(&self) -> &str {
        &self.auth_url
    }

    pub async fn complete(self, manager: &OAuthManager) -> Result<SecretString> {
        self.inner.block_until_done().await?;
        manager.load_saved_credential()
    }
}

#[derive(Clone)]
pub struct OAuthManager {
    pub config_dir: PathBuf,
    codex_home: PathBuf,
    saved_auth_available: Arc<Mutex<Option<bool>>>,
}

impl OAuthManager {
    pub fn new() -> Result<Self> {
        let config_dir = rara_config::ensure_rara_home_dir()?;
        Self::new_for_config_dir(config_dir)
    }

    pub fn new_for_config_dir(config_dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&config_dir)?;
        let codex_home = config_dir.join("codex-auth");
        std::fs::create_dir_all(&codex_home)?;
        Ok(Self {
            config_dir,
            codex_home,
            saved_auth_available: Arc::new(Mutex::new(None)),
        })
    }

    pub fn codex_issuer(&self) -> &'static str {
        ISSUER
    }

    pub fn client_id(&self) -> &'static str {
        CLIENT_ID
    }

    pub fn start_browser_login(&self, open_browser: bool) -> Result<BrowserLoginSession> {
        let mut options = self.server_options(open_browser);
        options.port = 0;
        let session = codex_run_login_server(options)?;
        Ok(BrowserLoginSession {
            auth_url: session.auth_url.clone(),
            inner: session,
        })
    }

    pub async fn request_device_code(&self) -> Result<DeviceCode> {
        let options = self.server_options(false);
        let code = codex_request_device_code(&options).await?;
        Ok(DeviceCode {
            verification_url: code.verification_url.clone(),
            user_code: code.user_code.clone(),
            inner: code,
        })
    }

    pub async fn complete_device_code_login(
        &self,
        device_code: &DeviceCode,
    ) -> Result<SecretString> {
        let options = self.server_options(false);
        codex_complete_device_code_login(options, device_code.inner.clone()).await?;
        self.load_saved_credential()
    }

    pub fn save_api_key(&self, api_key: &str) -> Result<SecretString> {
        codex_login_with_api_key(&self.codex_home, api_key, AuthCredentialsStoreMode::File)?;
        self.set_saved_auth_cache(true);
        self.load_saved_credential()
    }

    pub fn clear_saved_auth(&self) -> Result<bool> {
        let removed = codex_logout(
            &self.codex_home,
            AuthCredentialsStoreMode::File,
        )?;
        self.clear_saved_auth_cache();
        Ok(removed)
    }

    pub fn has_saved_auth(&self) -> Result<bool> {
        if let Some(cached) = self.saved_auth_cache() {
            return Ok(cached);
        }
        let Some(auth) = load_auth_dot_json(&self.codex_home, AuthCredentialsStoreMode::File)?
        else {
            self.set_saved_auth_cache(false);
            return Ok(false);
        };
        let has_api_key = auth
            .openai_api_key
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
        let has_access_token = auth
            .tokens
            .as_ref()
            .is_some_and(|tokens| !tokens.access_token.trim().is_empty());
        let has_saved_auth = has_api_key || has_access_token;
        self.set_saved_auth_cache(has_saved_auth);
        Ok(has_saved_auth)
    }

    pub fn read_api_key_from_stdin(&self) -> Result<SecretString> {
        eprintln!("Paste the Codex API key, then press Ctrl-D:");
        let mut input = String::new();
        std::io::stdin().read_to_string(&mut input)?;
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(anyhow!("API key input was empty"));
        }
        Ok(SecretString::from(trimmed.to_string()))
    }

    fn server_options(&self, open_browser: bool) -> ServerOptions {
        let mut options = ServerOptions::new(
            self.codex_home.clone(),
            CLIENT_ID.to_string(),
            None,
            AuthCredentialsStoreMode::File,
        );
        options.issuer = ISSUER.to_string();
        options.open_browser = open_browser;
        options
    }

    pub fn load_saved_credential(&self) -> Result<SecretString> {
        let auth = load_auth_dot_json(&self.codex_home, AuthCredentialsStoreMode::File)?
            .ok_or_else(|| anyhow!("Codex login finished but no credential was saved"))?;
        if let Some(api_key) = auth.openai_api_key.filter(|value| !value.trim().is_empty()) {
            self.set_saved_auth_cache(true);
            return Ok(SecretString::from(api_key));
        }
        if let Some(tokens) = auth
            .tokens
            .filter(|tokens| !tokens.access_token.trim().is_empty())
        {
            self.set_saved_auth_cache(true);
            return Ok(SecretString::from(tokens.access_token));
        }
        Err(anyhow!(
            "Codex login finished but auth storage did not contain an API key or access token"
        ))
    }

    pub fn invalidate_saved_auth_cache(&self) {
        self.clear_saved_auth_cache();
    }

    fn saved_auth_cache(&self) -> Option<bool> {
        self.saved_auth_available
            .lock()
            .ok()
            .and_then(|guard| *guard)
    }

    fn set_saved_auth_cache(&self, value: bool) {
        if let Ok(mut guard) = self.saved_auth_available.lock() {
            *guard = Some(value);
        }
    }

    fn clear_saved_auth_cache(&self) {
        if let Ok(mut guard) = self.saved_auth_available.lock() {
            *guard = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;
    use std::path::Path;
    use tempfile::tempdir;

    #[tokio::test]
    async fn browser_login_session_uses_codex_issuer_and_client_id() {
        let temp = tempdir().expect("tempdir");
        let manager =
            OAuthManager::new_for_config_dir(temp.path().join(".rara")).expect("oauth manager");
        let session = manager
            .start_browser_login(false)
            .expect("browser login session");

        assert!(session
            .auth_url()
            .starts_with("https://auth.openai.com/oauth/authorize?"));
        assert!(session.auth_url().contains(CLIENT_ID));
        session.inner.cancel();
    }

    #[test]
    fn save_api_key_persists_via_codex_auth_storage() {
        let temp = tempdir().expect("tempdir");
        let manager =
            OAuthManager::new_for_config_dir(temp.path().join(".rara")).expect("oauth manager");

        let stored = manager.save_api_key("sk-test-123").expect("save api key");

        assert_eq!(stored.expose_secret(), "sk-test-123");

        let auth = load_auth_dot_json(auth_path(&manager), AuthCredentialsStoreMode::File)
            .expect("load auth")
            .expect("auth file");
        assert_eq!(auth.openai_api_key.as_deref(), Some("sk-test-123"));
        assert!(auth.tokens.is_none());
    }

    #[test]
    fn load_saved_credential_prefers_api_key_then_access_token() {
        let temp = tempdir().expect("tempdir");
        let manager =
            OAuthManager::new_for_config_dir(temp.path().join(".rara")).expect("oauth manager");

        codex_login::save_auth(
            auth_path(&manager),
            &codex_login::AuthDotJson {
                auth_mode: None,
                openai_api_key: Some("sk-direct".into()),
                tokens: Some(codex_login::TokenData {
                    id_token: valid_id_token_info(),
                    access_token: "access".into(),
                    refresh_token: "refresh".into(),
                    account_id: None,
                }),
                last_refresh: None,
                agent_identity: None,
            },
            AuthCredentialsStoreMode::File,
        )
        .expect("save auth");

        let saved = manager.load_saved_credential().expect("load api key");
        assert_eq!(saved.expose_secret(), "sk-direct");

        codex_login::save_auth(
            auth_path(&manager),
            &codex_login::AuthDotJson {
                auth_mode: None,
                openai_api_key: None,
                tokens: Some(codex_login::TokenData {
                    id_token: valid_id_token_info(),
                    access_token: "access-only".into(),
                    refresh_token: "refresh".into(),
                    account_id: None,
                }),
                last_refresh: None,
                agent_identity: None,
            },
            AuthCredentialsStoreMode::File,
        )
        .expect("save token auth");

        let saved = manager.load_saved_credential().expect("load access token");
        assert_eq!(saved.expose_secret(), "access-only");
    }

    #[test]
    fn logout_clears_codex_auth_storage() {
        let temp = tempdir().expect("tempdir");
        let manager =
            OAuthManager::new_for_config_dir(temp.path().join(".rara")).expect("oauth manager");
        manager
            .save_api_key("sk-test-logout")
            .expect("save api key");

        let removed = manager.clear_saved_auth().expect("clear auth");
        assert!(removed);

        let auth = load_auth_dot_json(auth_path(&manager), AuthCredentialsStoreMode::File)
            .expect("load auth after logout");
        assert!(auth.is_none());
    }

    #[test]
    fn has_saved_auth_detects_api_key_and_access_token_storage() {
        let temp = tempdir().expect("tempdir");
        let manager =
            OAuthManager::new_for_config_dir(temp.path().join(".rara")).expect("oauth manager");

        assert!(!manager.has_saved_auth().expect("no auth"));

        manager.save_api_key("sk-test-123").expect("save api key");
        assert!(manager.has_saved_auth().expect("api key auth"));

        manager.clear_saved_auth().expect("clear auth");
        assert!(!manager.has_saved_auth().expect("cleared auth"));

        codex_login::save_auth(
            auth_path(&manager),
            &codex_login::AuthDotJson {
                auth_mode: None,
                openai_api_key: None,
                tokens: Some(codex_login::TokenData {
                    id_token: valid_id_token_info(),
                    access_token: "access-only".into(),
                    refresh_token: "refresh".into(),
                    account_id: None,
                }),
                last_refresh: None,
                agent_identity: None,
            },
            AuthCredentialsStoreMode::File,
        )
        .expect("save token auth");

        manager.invalidate_saved_auth_cache();
        assert!(manager.has_saved_auth().expect("token auth"));
    }

    #[test]
    fn load_saved_credential_rejects_blank_values() {
        let temp = tempdir().expect("tempdir");
        let manager =
            OAuthManager::new_for_config_dir(temp.path().join(".rara")).expect("oauth manager");

        codex_login::save_auth(
            auth_path(&manager),
            &codex_login::AuthDotJson {
                auth_mode: None,
                openai_api_key: Some("   ".into()),
                tokens: Some(codex_login::TokenData {
                    id_token: valid_id_token_info(),
                    access_token: "   ".into(),
                    refresh_token: "refresh".into(),
                    account_id: None,
                }),
                last_refresh: None,
                agent_identity: None,
            },
            AuthCredentialsStoreMode::File,
        )
        .expect("save auth");

        let err = manager
            .load_saved_credential()
            .expect_err("blank credentials should be rejected");
        assert!(err
            .to_string()
            .contains("did not contain an API key or access token"));
    }

    #[test]
    fn has_saved_auth_refreshes_after_cache_invalidation() {
        let temp = tempdir().expect("tempdir");
        let manager =
            OAuthManager::new_for_config_dir(temp.path().join(".rara")).expect("oauth manager");

        assert!(!manager.has_saved_auth().expect("no auth"));

        codex_login::save_auth(
            auth_path(&manager),
            &codex_login::AuthDotJson {
                auth_mode: None,
                openai_api_key: Some("sk-direct".into()),
                tokens: None,
                last_refresh: None,
                agent_identity: None,
            },
            AuthCredentialsStoreMode::File,
        )
        .expect("save auth");

        assert!(!manager.has_saved_auth().expect("stale false cache"));
        manager.invalidate_saved_auth_cache();
        assert!(manager.has_saved_auth().expect("refreshed auth"));
    }

    fn auth_path(manager: &OAuthManager) -> &Path {
        manager.codex_home.as_path()
    }

    fn valid_id_token_info() -> codex_login::token_data::IdTokenInfo {
        codex_login::token_data::parse_chatgpt_jwt_claims("eyJhbGciOiJub25lIn0.e30.signature")
            .expect("valid id token")
    }
}
