use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub call_id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolCallOutputStatus {
    Success,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCallOutput {
    pub call_id: String,
    pub output: Value,
    pub status: ToolCallOutputStatus,
}

impl ToolCallOutput {
    pub fn success(call_id: impl Into<String>, output: Value) -> Self {
        Self {
            call_id: call_id.into(),
            output,
            status: ToolCallOutputStatus::Success,
        }
    }

    pub fn error(call_id: impl Into<String>, output: Value) -> Self {
        Self {
            call_id: call_id.into(),
            output,
            status: ToolCallOutputStatus::Error,
        }
    }
}
