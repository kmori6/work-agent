use crate::domain::error::tool_approval_repository_error::ToolApprovalRepositoryError;
use crate::domain::model::tool_approval::ToolApproval;
use async_trait::async_trait;

#[async_trait]
pub trait ToolApprovalRepository: Send + Sync {
    async fn record(&self, approval: ToolApproval) -> Result<(), ToolApprovalRepositoryError>;
}
