use crate::application::error::llm_client_error::LlmClientError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("failed to call llm provider: {0}")]
    LlmClient(#[from] LlmClientError),

    #[error("agent exceeded maximum tool iterations: {0}")]
    MaxToolIterations(usize),
}
