use std::sync::Arc;

use crate::agent::Agent;
use crate::llm::LlmBackend;
use crate::sandbox::SandboxManager;
use crate::session::SessionManager;
use crate::skill::SkillManager;
use crate::tool::ToolManager;
use crate::tools::agent::{AgentTool, ExploreAgentTool, PlanAgentTool, TeamCreateTool};
use crate::tools::bash::BashTool;
use crate::tools::context::RetrieveSessionContextTool;
use crate::tools::file::{ListFilesTool, ReadFileTool, ReplaceTool, WriteFileTool};
use crate::tools::patch::ApplyPatchTool;
use crate::tools::search::{GlobTool, GrepTool};
use crate::tools::skill::SkillTool;
use crate::tools::vector::{RememberExperienceTool, RetrieveExperienceTool};
use crate::tools::web::WebFetchTool;
use crate::tools::workspace::UpdateProjectMemoryTool;
use crate::vectordb::VectorDB;
use crate::workspace::WorkspaceMemory;

pub(super) async fn rebuild_agent_with_progress(
    config: &crate::config::RaraConfig,
    progress: Option<crate::local_backend::LocalProgressReporter>,
) -> anyhow::Result<Agent> {
    let backend = crate::build_backend_with_progress(config, progress).await?;
    let backend_arc: Arc<dyn LlmBackend> = backend.into();

    let vdb = Arc::new(VectorDB::new("data/lancedb"));
    let session_manager = Arc::new(SessionManager::new()?);
    let workspace = Arc::new(WorkspaceMemory::new()?);
    let sandbox_manager = Arc::new(SandboxManager::new()?);

    let mut skill_manager = SkillManager::new();
    let _ = skill_manager.load_all();
    let skill_manager_arc = Arc::new(skill_manager);

    let tool_manager = create_full_tool_manager(
        backend_arc.clone(),
        vdb.clone(),
        session_manager.clone(),
        workspace.clone(),
        sandbox_manager.clone(),
        skill_manager_arc,
        crate::prompt::PromptRuntimeConfig::from_config(config),
    );

    let mut agent = Agent::new(tool_manager, backend_arc, vdb, session_manager, workspace);
    agent.set_prompt_config(crate::prompt::PromptRuntimeConfig::from_config(config));
    Ok(agent)
}

fn create_full_tool_manager(
    backend: Arc<dyn LlmBackend>,
    vdb: Arc<VectorDB>,
    session_manager: Arc<SessionManager>,
    workspace: Arc<WorkspaceMemory>,
    sandbox: Arc<SandboxManager>,
    skill_manager: Arc<SkillManager>,
    prompt_config: crate::prompt::PromptRuntimeConfig,
) -> ToolManager {
    let mut tm = ToolManager::new();
    tm.register(Box::new(BashTool {
        sandbox: sandbox.clone(),
    }));
    tm.register(Box::new(ReadFileTool));
    tm.register(Box::new(ApplyPatchTool));
    tm.register(Box::new(WriteFileTool));
    tm.register(Box::new(ListFilesTool));
    tm.register(Box::new(ReplaceTool));
    tm.register(Box::new(WebFetchTool));
    tm.register(Box::new(GlobTool));
    tm.register(Box::new(GrepTool));
    tm.register(Box::new(RememberExperienceTool {
        backend: backend.clone(),
        db_uri: "data/lancedb".into(),
    }));
    tm.register(Box::new(RetrieveExperienceTool {
        backend: backend.clone(),
        db_uri: "data/lancedb".into(),
    }));
    tm.register(Box::new(RetrieveSessionContextTool {
        backend: backend.clone(),
        vdb: vdb.clone(),
        session_manager: session_manager.clone(),
    }));
    tm.register(Box::new(UpdateProjectMemoryTool {
        workspace: workspace.clone(),
    }));
    tm.register(Box::new(SkillTool {
        skill_manager: skill_manager.clone(),
    }));
    tm.register(Box::new(AgentTool {
        backend: backend.clone(),
        vdb: vdb.clone(),
        session_manager: session_manager.clone(),
        workspace: workspace.clone(),
        prompt_config: prompt_config.clone(),
    }));
    tm.register(Box::new(ExploreAgentTool {
        backend: backend.clone(),
        vdb: vdb.clone(),
        session_manager: session_manager.clone(),
        workspace: workspace.clone(),
        prompt_config: prompt_config.clone(),
    }));
    tm.register(Box::new(PlanAgentTool {
        backend: backend.clone(),
        vdb: vdb.clone(),
        session_manager: session_manager.clone(),
        workspace: workspace.clone(),
        prompt_config,
    }));
    tm.register(Box::new(TeamCreateTool {
        backend,
        vdb,
        session_manager,
        workspace,
    }));
    tm
}
