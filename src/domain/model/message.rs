use crate::domain::model::role::Role;
use crate::domain::model::tool::ToolCall;
use crate::domain::model::tool::ToolResultMessage;

#[derive(Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub content: MessageContent,
}

#[derive(Debug, Clone)]
pub enum MessageContent {
    Text(String),
    ToolCall {
        text: Option<String>,
        tool_calls: Vec<ToolCall>,
    },
    ToolResult(ToolResultMessage),
}

impl Message {
    pub fn text(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: MessageContent::Text(content.into()),
        }
    }

    pub fn tool_result(role: Role, tool_result: ToolResultMessage) -> Self {
        Self {
            role,
            content: MessageContent::ToolResult(tool_result),
        }
    }

    pub fn tool_call(text: Option<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: Role::Assistant,
            content: MessageContent::ToolCall { text, tool_calls },
        }
    }
}
