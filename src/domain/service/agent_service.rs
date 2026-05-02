use crate::domain::error::agent_error::AgentError;
use crate::domain::model::message::{Message, MessageContent};
use crate::domain::model::role::Role;
use crate::domain::port::llm_provider::{LlmProvider, LlmResponse};
use crate::domain::service::tool_service::ToolService;

const DEFAULT_MODEL: &str = "global.anthropic.claude-sonnet-4-6";

pub struct AgentService<L> {
    llm_provider: L,
    tool_service: ToolService,
    model: String,
}

impl<L: LlmProvider> AgentService<L> {
    pub fn new(llm_provider: L, tool_service: ToolService) -> Self {
        Self {
            llm_provider,
            tool_service,
            model: DEFAULT_MODEL.to_string(),
        }
    }

    pub fn tool_service(&self) -> &ToolService {
        &self.tool_service
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub async fn llm_step(
        &self,
        instruction: String,
        messages: Vec<Message>,
    ) -> Result<LlmResponse, AgentError> {
        let instructions = Message::new(
            Role::System,
            vec![MessageContent::InputText { text: instruction }],
        )?;

        let mut llm_messages = Vec::with_capacity(messages.len() + 1);
        llm_messages.push(instructions);
        llm_messages.extend(messages);

        self.llm_provider
            .response_with_tool(llm_messages, self.tool_service.specs(), &self.model)
            .await
            .map_err(AgentError::LlmProvider)
    }
}
