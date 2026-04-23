use crate::application::error::tool_execution_rule_usecase_error::ToolExecutionRuleUsecaseError;
use crate::domain::model::tool_execution_rule::{ToolExecutionRule, ToolExecutionRuleAction};
use crate::domain::repository::tool_execution_rule_repository::ToolExecutionRuleRepository;

pub struct ToolExecutionRuleUsecase<R> {
    repository: R,
}

impl<R> ToolExecutionRuleUsecase<R>
where
    R: ToolExecutionRuleRepository,
{
    pub fn new(repository: R) -> Self {
        Self { repository }
    }

    pub async fn list(&self) -> Result<Vec<ToolExecutionRule>, ToolExecutionRuleUsecaseError> {
        Ok(self.repository.list_all().await?)
    }

    pub async fn set(
        &self,
        tool_name: String,
        action: ToolExecutionRuleAction,
    ) -> Result<(), ToolExecutionRuleUsecaseError> {
        self.repository
            .save(ToolExecutionRule { tool_name, action })
            .await?;

        Ok(())
    }
}
