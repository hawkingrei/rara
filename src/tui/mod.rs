mod app_event;
mod auth_mode_picker;
mod command;
mod custom_terminal;
mod event_dispatch;
mod event_loop;
mod event_stream;
mod highlight;
mod insert_history;
mod interaction_text;
mod keymap;
mod line_utils;
mod markdown;
mod markdown_render;
mod markdown_stream;
mod plan_display;
mod provider_flow;
mod queued_input;
mod render;
mod runtime;
mod session_restore;
mod state;
mod submit;
mod terminal_event;
mod terminal_ui;
#[cfg(test)]
mod tests;
mod tool_text;

pub(crate) use self::keymap::map_key_to_event;
pub(crate) use self::session_restore::provider_requires_api_key;
pub(crate) use self::terminal_ui::is_ssh_session;

pub(crate) use self::event_dispatch::dispatch_event;
pub(crate) use self::submit::{
    PendingPlanApprovalAction, classify_pending_plan_approval_input, handle_submit,
};

pub use self::event_loop::{StartupResumeTarget, run_tui};
