use crate::domain::error::llm_provider_error::LlmProviderError;
use crate::domain::error::message_error::MessageError;
use crate::domain::error::tool_error::ToolError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("failed to call llm provider: {0}")]
    LlmProvider(#[from] LlmProviderError),

    #[error("invalid message: {0}")]
    Message(#[from] MessageError),

    #[error("failed to handle tool execution: {0}")]
    ToolCall(#[from] ToolError),

    #[error("agent exceeded maximum tool iterations: {0}")]
    MaxToolIterations(usize),
}
