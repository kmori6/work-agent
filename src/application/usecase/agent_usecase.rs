use crate::application::error::agent_usecase_error::AgentUsecaseError;
use crate::domain::port::llm_provider::LlmProvider;
use crate::domain::service::agent_service::AgentProgressEvent;
use crate::domain::service::agent_service::AgentService;

#[derive(Debug)]
pub struct HandleAgentInput {
    pub user_input: String,
}

#[derive(Debug)]
pub struct HandleAgentOutput {
    pub reply: Vec<AgentEvent>,
}

#[derive(Debug)]
pub enum AgentEvent {
    AssistantMessage(String),
}

pub struct AgentUsecase<L> {
    agent_service: AgentService<L>,
}

impl<L: LlmProvider> AgentUsecase<L> {
    pub fn new(agent_service: AgentService<L>) -> Self {
        Self { agent_service }
    }

    pub async fn handle(
        &self,
        input: HandleAgentInput,
    ) -> Result<HandleAgentOutput, AgentUsecaseError> {
        let result = self.agent_service.run(input.user_input).await?;

        Ok(HandleAgentOutput {
            reply: vec![AgentEvent::AssistantMessage(result.final_text)],
        })
    }

    pub async fn handle_with_progress<F>(
        &self,
        input: HandleAgentInput,
        emit: F,
    ) -> Result<HandleAgentOutput, AgentUsecaseError>
    where
        F: FnMut(AgentProgressEvent),
    {
        let result = self
            .agent_service
            .run_with_progress(input.user_input, emit)
            .await?;

        Ok(HandleAgentOutput {
            reply: vec![AgentEvent::AssistantMessage(result.final_text)],
        })
    }
}
