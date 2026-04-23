use crate::domain::error::tool_execution_rule_repository_error::ToolExecutionRuleRepositoryError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ToolExecutionRuleUsecaseError {
    #[error("failed to access tool execution rule repository: {0}")]
    Repository(#[from] ToolExecutionRuleRepositoryError),
}
