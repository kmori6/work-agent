use async_trait::async_trait;
use sqlx::PgPool;

use crate::domain::error::tool_approval_repository_error::ToolApprovalRepositoryError;
use crate::domain::model::tool_approval::ToolApproval;
use crate::domain::repository::tool_approval_repository::ToolApprovalRepository;

#[derive(Clone)]
pub struct PostgresToolApprovalRepository {
    pool: PgPool,
}

impl PostgresToolApprovalRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn map_sqlx_error(err: sqlx::Error) -> ToolApprovalRepositoryError {
    ToolApprovalRepositoryError::Unexpected(err.to_string())
}

#[async_trait]
impl ToolApprovalRepository for PostgresToolApprovalRepository {
    async fn record(&self, approval: ToolApproval) -> Result<(), ToolApprovalRepositoryError> {
        let ToolApproval {
            session_id,
            tool_call_id,
            tool_name,
            arguments,
            decision,
        } = approval;

        sqlx::query(
            r#"
            INSERT INTO tool_call_approvals (
              session_id, tool_call_id, tool_name, arguments, decision
            )
            VALUES ($1, $2, $3, $4, $5)
            "#,
        )
        .bind(session_id)
        .bind(tool_call_id)
        .bind(tool_name)
        .bind(arguments)
        .bind(decision.as_str())
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        Ok(())
    }
}
