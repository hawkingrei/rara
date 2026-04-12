use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::JoinHandle;

use crate::agent::Agent;
use crate::config::{ConfigManager, RaraConfig};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HelpTab {
    General,
    Commands,
    Runtime,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Overlay {
    Welcome,
    Help(HelpTab),
    CommandPalette,
    Status,
    Setup,
    ModelPicker,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LocalCommandKind {
    Help,
    Status,
    Clear,
    Setup,
    Model,
    Login,
}

pub struct LocalCommand {
    pub kind: LocalCommandKind,
    pub arg: Option<String>,
}

pub struct CommandSpec {
    pub category: &'static str,
    pub name: &'static str,
    pub usage: &'static str,
    pub summary: &'static str,
    pub detail: &'static str,
}

#[derive(Default, Clone)]
pub struct RuntimeSnapshot {
    pub cwd: String,
    pub branch: String,
    pub session_id: String,
    pub history_len: usize,
    pub total_input_tokens: u32,
    pub total_output_tokens: u32,
}

pub enum TaskKind {
    Query,
    Rebuild,
    OAuth,
}

pub enum TaskCompletion {
    Query {
        agent: Agent,
        result: anyhow::Result<()>,
    },
    Rebuild {
        result: anyhow::Result<Agent>,
    },
    OAuth {
        result: anyhow::Result<String>,
    },
}

pub enum TuiEvent {
    Transcript {
        role: &'static str,
        message: String,
    },
}

pub struct RunningTask {
    pub kind: TaskKind,
    pub receiver: UnboundedReceiver<TuiEvent>,
    pub handle: JoinHandle<TaskCompletion>,
}

pub const LOCAL_MODEL_PRESETS: [(&str, &str, &str); 3] = [
    ("Gemma 4 E4B", "gemma4", "gemma4-e4b"),
    ("Gemma 4 E2B", "gemma4", "gemma4-e2b"),
    ("Qwn3 8B", "qwn3", "qwn3-8b"),
];

pub struct TuiApp {
    pub input: String,
    pub transcript: Vec<(String, String)>,
    pub overlay: Option<Overlay>,
    pub config: RaraConfig,
    pub config_manager: ConfigManager,
    pub setup_status: Option<String>,
    pub notice: Option<String>,
    pub snapshot: RuntimeSnapshot,
    pub model_picker_idx: usize,
    pub command_palette_idx: usize,
    pub running_task: Option<RunningTask>,
}

impl TuiApp {
    pub fn new(cm: ConfigManager) -> Self {
        let cfg = cm.load();
        let overlay = if cfg.api_key.is_none() && super::provider_requires_api_key(&cfg.provider) {
            Some(Overlay::Setup)
        } else {
            Some(Overlay::Welcome)
        };
        let model_picker_idx = selected_preset_idx_for_config(&cfg);
        Self {
            input: String::new(),
            transcript: Vec::new(),
            overlay,
            config: cfg,
            config_manager: cm,
            setup_status: None,
            notice: None,
            snapshot: RuntimeSnapshot::default(),
            model_picker_idx,
            command_palette_idx: 0,
            running_task: None,
        }
    }

    pub fn is_busy(&self) -> bool {
        self.running_task.is_some()
    }

    pub fn current_model_label(&self) -> &str {
        self.config.model.as_deref().unwrap_or("-")
    }

    pub fn selected_preset_idx(&self) -> usize {
        selected_preset_idx_for_config(&self.config)
    }

    pub fn select_local_model(&mut self, idx: usize) {
        let (_, provider, model) = LOCAL_MODEL_PRESETS[idx];
        self.model_picker_idx = idx;
        self.config.provider = provider.to_string();
        self.config.model = Some(model.to_string());
        self.config.revision = Some("main".to_string());
    }

    pub fn cycle_local_model(&mut self) {
        let next = (self.selected_preset_idx() + 1) % LOCAL_MODEL_PRESETS.len();
        self.select_local_model(next);
    }

    pub fn sync_snapshot(&mut self, agent: &Agent) {
        let (cwd, branch) = agent.workspace.get_env_info();
        self.snapshot = RuntimeSnapshot {
            cwd,
            branch,
            session_id: agent.session_id.clone(),
            history_len: agent.history.len(),
            total_input_tokens: agent.total_input_tokens,
            total_output_tokens: agent.total_output_tokens,
        };
    }

    pub fn push_entry(&mut self, role: &'static str, message: impl Into<String>) {
        self.transcript.push((role.into(), message.into()));
    }

    pub fn push_notice(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.notice = Some(message.clone());
        self.push_entry("System", message);
    }

    pub fn reset_transcript(&mut self) {
        self.transcript.clear();
        self.notice = Some("Cleared local transcript view.".into());
    }

    pub fn open_overlay(&mut self, overlay: Overlay) {
        if matches!(overlay, Overlay::CommandPalette) {
            self.command_palette_idx = 0;
        }
        if matches!(overlay, Overlay::ModelPicker) {
            self.model_picker_idx = self.selected_preset_idx();
        }
        self.overlay = Some(overlay);
    }

    pub fn close_overlay(&mut self) {
        self.overlay = None;
    }
}

pub fn selected_preset_idx_for_config(config: &RaraConfig) -> usize {
    LOCAL_MODEL_PRESETS
        .iter()
        .position(|(_, provider, model)| {
            config.provider == *provider && config.model.as_deref() == Some(*model)
        })
        .unwrap_or(0)
}
