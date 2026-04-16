use crate::domain::error::agent_error::AgentError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentUsecaseError {
    #[error("failed to execute agent use case: {0}")]
    Agent(#[from] AgentError),
}
