use crate::llm::LlmBackend;
use crate::tool::ToolManager;
use agent_client_protocol::Error;
use agent_client_protocol::schema::{
    AuthenticateRequest, AuthenticateResponse, CancelNotification, Implementation,
    InitializeRequest, InitializeResponse, NewSessionRequest, NewSessionResponse, PromptRequest,
    PromptResponse, ProtocolVersion, SessionId, StopReason,
};

pub struct RaraAcpAgent {
    pub tool_manager: ToolManager,
    pub backend_builder: Box<dyn Fn() -> Box<dyn LlmBackend> + Send + Sync>,
}

impl RaraAcpAgent {
    pub async fn initialize(&self, _: InitializeRequest) -> Result<InitializeResponse, Error> {
        Ok(InitializeResponse::new(ProtocolVersion::V1)
            .agent_info(Implementation::new("rara", "0.1.0")))
    }

    pub async fn authenticate(
        &self,
        _: AuthenticateRequest,
    ) -> Result<AuthenticateResponse, Error> {
        Err(Error::method_not_found())
    }

    pub async fn new_session(&self, _: NewSessionRequest) -> Result<NewSessionResponse, Error> {
        Ok(NewSessionResponse::new(SessionId::new(
            "default".to_string(),
        )))
    }

    pub async fn prompt(&self, _: PromptRequest) -> Result<PromptResponse, Error> {
        Ok(PromptResponse::new(StopReason::EndTurn))
    }

    pub async fn cancel(&self, _: CancelNotification) -> Result<(), Error> {
        Ok(())
    }
}

pub async fn run_acp_stdio(_agent: RaraAcpAgent) -> anyhow::Result<()> {
    // Runner implementation depends on the exact crate structure
    Ok(())
}
