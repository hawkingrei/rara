pub mod specs;
pub mod status;

#[cfg(test)]
mod tests;

pub use self::specs::{
    COMMAND_SPECS, command_detail_text, command_spec_by_index, command_spec_by_name,
    general_help_text, help_text, matching_commands, palette_command_by_index, palette_commands,
    parse_local_command, quick_actions_text, recent_command_specs, recommended_commands,
};
pub use self::status::{
    api_key_status, current_turn_preview, download_status_text, is_local_provider, model_help_text,
    recent_transcript_preview, status_context_text, status_prompt_sources_text,
    status_resources_text, status_runtime_text, status_workspace_text,
};
