use crate::domain::error::tool_error::ToolError;
use crate::domain::model::tool_call::{ToolCallOutputStatus, ToolSpec};
pub use crate::domain::model::tool_execution_policy::ToolExecutionPolicy;
use async_trait::async_trait;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutput {
    pub output: Value,
    pub status: ToolCallOutputStatus,
}

impl ToolOutput {
    pub fn success(output: Value) -> Self {
        Self {
            output,
            status: ToolCallOutputStatus::Success,
        }
    }

    pub fn error(output: Value) -> Self {
        Self {
            output,
            status: ToolCallOutputStatus::Error,
        }
    }
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

    /// Returns the tool's default execution policy for this invocation.
    ///
    /// Implementations may inspect `arguments` to choose a stricter policy for
    /// risky operations. Read-only tools can usually keep the default `Auto`.
    fn execution_policy(&self, _arguments: &Value) -> ToolExecutionPolicy {
        ToolExecutionPolicy::Auto
    }

    async fn execute(&self, arguments: Value) -> Result<ToolOutput, ToolError>;
}
