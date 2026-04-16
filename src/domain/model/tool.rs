#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolExecutionResult {
    pub output: serde_json::Value,
    pub is_error: bool,
}

impl ToolExecutionResult {
    pub fn success(output: serde_json::Value) -> Self {
        Self {
            output,
            is_error: false,
        }
    }

    pub fn error(output: serde_json::Value) -> Self {
        Self {
            output,
            is_error: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolResultMessage {
    pub tool_call_id: String,
    pub output: serde_json::Value,
    pub is_error: bool,
}

impl ToolResultMessage {
    pub fn from_execution(
        tool_call_id: impl Into<String>,
        execution_result: ToolExecutionResult,
    ) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            output: execution_result.output,
            is_error: execution_result.is_error,
        }
    }
}
