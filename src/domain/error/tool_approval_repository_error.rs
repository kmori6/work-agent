use thiserror::Error;

#[derive(Debug, Error)]
pub enum ToolApprovalRepositoryError {
    #[error("failed to access tool approval repository: {0}")]
    Unexpected(String),
}
