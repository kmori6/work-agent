use crate::domain::error::tool_error::ToolError;
use crate::domain::model::tool::{ToolExecutionResult, ToolSpec};
use async_trait::async_trait;

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> serde_json::Value;

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: self.parameters(),
        }
    }

    async fn execute(&self, arguments: serde_json::Value)
    -> Result<ToolExecutionResult, ToolError>;
}
