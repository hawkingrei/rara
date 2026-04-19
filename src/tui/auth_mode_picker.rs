use super::command::api_key_status;
use super::state::TuiApp;

pub(crate) struct AuthModePickerView {
    pub(crate) intro: String,
    pub(crate) lines: Vec<String>,
    pub(crate) footer: &'static str,
}

pub(crate) fn build_auth_mode_picker_view(app: &TuiApp, ssh_session: bool) -> AuthModePickerView {
    let ssh_hint = if ssh_session {
        "\n\nSSH session detected. Browser login on a remote shell usually cannot complete the localhost callback. Device-code login or API key is recommended in SSH/headless sessions."
    } else {
        ""
    };
    let intro = format!(
        "Codex needs authentication before this preset can be used.\n\n\
         Choose one auth mode below.{ssh_hint}"
    );
    let options = [
        (
            "Browser login",
            "Best for local desktop sessions with a localhost callback.",
        ),
        (
            "Device code",
            "Best for SSH/headless sessions. Open the URL elsewhere and enter the one-time code.",
        ),
        (
            "API key",
            "Paste an existing Codex-compatible API key and save it locally.",
        ),
        (
            "Logout",
            "Clear the saved provider credential and rebuild the current codex backend.",
        ),
    ];
    let mut lines = vec![
        format!("Current model: {}", app.current_model_label()),
        "Provider: codex".to_string(),
        format!("Credential status: {}", api_key_status(&app.config)),
        String::new(),
    ];
    for (idx, (title, detail)) in options.iter().enumerate() {
        let marker = if idx == app.auth_mode_idx { ">" } else { " " };
        lines.push(format!("{marker} {title}"));
        lines.push(format!("    {detail}"));
    }

    let footer = if ssh_session {
        "Up/Down move  Enter choose  number keys jump  default: device code  Esc back"
    } else {
        "Up/Down move  Enter choose  number keys jump  default: browser login  Esc back"
    };

    AuthModePickerView {
        intro,
        lines,
        footer,
    }
}
