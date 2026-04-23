use crate::domain::error::agent_error::AgentError;
use crate::domain::error::chat_repository_error::ChatRepositoryError;
use crate::domain::error::token_usage_repository_error::TokenUsageRepositoryError;
use crate::domain::error::tool_approval_repository_error::ToolApprovalRepositoryError;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum AgentUsecaseError {
    #[error("failed to execute agent use case: {0}")]
    Agent(#[from] AgentError),

    #[error("failed to access chat repository: {0}")]
    ChatRepository(#[from] ChatRepositoryError),

    #[error("failed to access token usage repository: {0}")]
    TokenUsageRepository(#[from] TokenUsageRepositoryError),

    #[error("tool confirmation is already pending for session: {0}")]
    ApprovalPending(Uuid),

    #[error("no approval is pending for session: {0}")]
    ApprovalNotPending(Uuid),

    #[error("failed to access tool approval repository: {0}")]
    ToolApprovalRepository(#[from] ToolApprovalRepositoryError),
}
