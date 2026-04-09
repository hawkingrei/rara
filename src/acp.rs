use agent_client_protocol::{
    Agent, InitializeRequest, InitializeResponse, NewSessionRequest, NewSessionResponse,
    PromptRequest, PromptResponse, ProtocolVersion, Implementation, AuthenticateRequest,
    AuthenticateResponse, CancelNotification, Error
};
use async_trait::async_trait;
use crate::tool::ToolManager;
use crate::llm::LlmBackend;

pub struct RaraAcpAgent { 
    pub tool_manager: ToolManager, 
    pub backend_builder: Box<dyn Fn() -> Box<dyn LlmBackend> + Send + Sync> 
}

#[async_trait(?Send)]
impl Agent for RaraAcpAgent {
    async fn initialize(&self, _: InitializeRequest) -> Result<InitializeResponse, Error> {
        Ok(InitializeResponse::new(ProtocolVersion::V1)
            .agent_info(Implementation::new("rara", "0.1.0")))
    }

    async fn authenticate(&self, _: AuthenticateRequest) -> Result<AuthenticateResponse, Error> {
        Err(Error::method_not_found())
    }

    async fn new_session(&self, _: NewSessionRequest) -> Result<NewSessionResponse, Error> {
        Ok(NewSessionResponse::new(agent_client_protocol::SessionId::new("default".to_string())))
    }

    async fn prompt(&self, _: PromptRequest) -> Result<PromptResponse, Error> {
        Ok(PromptResponse::new(agent_client_protocol::StopReason::EndTurn))
    }

    async fn cancel(&self, _: CancelNotification) -> Result<(), Error> {
        Ok(())
    }
}

pub async fn run_acp_stdio(_agent: RaraAcpAgent) -> anyhow::Result<()> {
    // Runner implementation depends on the exact crate structure
    Ok(())
}
