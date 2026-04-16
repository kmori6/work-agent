use crate::domain::model::tool::{ToolCall, ToolExecutionResult, ToolResultMessage, ToolSpec};
use crate::domain::port::tool::Tool;
use serde_json::json;
use std::sync::Arc;

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

    pub async fn execute(&self, call: ToolCall) -> ToolResultMessage {
        let Some(tool) = self.tools.iter().find(|tool| tool.name() == call.name) else {
            return ToolResultMessage::from_execution(
                call.id,
                ToolExecutionResult::error(json!({
                    "message": format!("unknown tool: {}", call.name)
                })),
            );
        };

        match tool.execute(call.arguments).await {
            Ok(result) => ToolResultMessage::from_execution(call.id, result),
            Err(err) => ToolResultMessage::from_execution(
                call.id,
                ToolExecutionResult::error(json!({
                    "message": err.to_string()
                })),
            ),
        }
    }
}
