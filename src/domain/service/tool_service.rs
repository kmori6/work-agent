use crate::domain::error::tool_error::ToolError;
use crate::domain::model::tool::{ToolCall, ToolExecutionResult, ToolSpec};
use crate::domain::port::tool::Tool;
use crate::domain::port::tool::ToolExecutionPolicy;
use std::sync::Arc;

#[derive(Clone)]
pub struct ToolExecutor {
    tools: Vec<Arc<dyn Tool>>,
}

impl ToolExecutor {
    pub fn new(tools: Vec<Arc<dyn Tool>>) -> Self {
        Self { tools }
    }

    pub fn specs(&self) -> Vec<ToolSpec> {
        self.tools.iter().map(|tool| tool.spec()).collect()
    }

    pub fn check_execution_policy(
        &self,
        call: &ToolCall,
    ) -> Result<ToolExecutionPolicy, ToolError> {
        self.tools
            .iter()
            .find(|tool| tool.name() == call.name)
            .map(|tool| tool.execution_policy(&call.arguments))
            .ok_or_else(|| ToolError::UnknownTool(call.name.clone()))
    }

    pub async fn execute(&self, call: ToolCall) -> Result<ToolExecutionResult, ToolError> {
        let tool = self
            .tools
            .iter()
            .find(|tool| tool.name() == call.name)
            .ok_or_else(|| ToolError::UnknownTool(call.name.clone()))?;

        tool.execute(call.arguments).await
    }
}
