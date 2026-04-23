use crate::domain::error::tool_error::ToolError;
use crate::domain::model::tool::{ToolExecutionResult, ToolSpec};
use async_trait::async_trait;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolExecutionPolicy {
    Auto,             // execute automatically
    Ask,              // ask the user for confirmation before executing
    ConfirmEveryTime, // confirm every time before executing
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> Value;

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: self.parameters(),
        }
    }

    fn execution_policy(&self, _arguments: &Value) -> ToolExecutionPolicy {
        ToolExecutionPolicy::Auto
    }

    async fn execute(&self, arguments: Value) -> Result<ToolExecutionResult, ToolError>;
}
