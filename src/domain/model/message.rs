use crate::domain::error::message_error::MessageError;
use crate::domain::model::input_file::InputFile;
use crate::domain::model::input_image::InputImage;
use crate::domain::model::role::Role;
use crate::domain::model::tool_call::{ToolCall, ToolCallOutput};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageType {
    Message,
    Tool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MessageContent {
    InputText(String),
    InputImage(InputImage),
    InputFile(InputFile),
    OutputText(String),
    ToolCall(ToolCall),
    ToolCallOutput(ToolCallOutput),
}

impl MessageContent {
    pub fn message_type(&self) -> MessageType {
        match self {
            Self::InputText(_) | Self::InputImage(_) | Self::InputFile(_) | Self::OutputText(_) => {
                MessageType::Message
            }

            Self::ToolCall(_) | Self::ToolCallOutput(_) => MessageType::Tool,
        }
    }

    pub fn is_persistable(&self) -> bool {
        !matches!(self, Self::InputImage(_) | Self::InputFile(_))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub role: Role,
    pub contents: Vec<MessageContent>,
}

impl Message {
    pub fn new(role: Role, contents: Vec<MessageContent>) -> Result<Self, MessageError> {
        let message = Self { role, contents };
        message.message_type()?;
        Ok(message)
    }

    pub fn input_text(text: impl Into<String>) -> Result<Self, MessageError> {
        Self::new(Role::User, vec![MessageContent::InputText(text.into())])
    }

    pub fn output_text(text: impl Into<String>) -> Result<Self, MessageError> {
        Self::new(
            Role::Assistant,
            vec![MessageContent::OutputText(text.into())],
        )
    }

    pub fn tool_calls(tool_calls: Vec<ToolCall>) -> Result<Self, MessageError> {
        Self::new(
            Role::Assistant,
            tool_calls
                .into_iter()
                .map(MessageContent::ToolCall)
                .collect(),
        )
    }

    pub fn tool_call_outputs(outputs: Vec<ToolCallOutput>) -> Result<Self, MessageError> {
        Self::new(
            Role::User,
            outputs
                .into_iter()
                .map(MessageContent::ToolCallOutput)
                .collect(),
        )
    }

    pub fn message_type(&self) -> Result<MessageType, MessageError> {
        let first = self.contents.first().ok_or(MessageError::EmptyContents)?;

        let message_type = first.message_type();

        if self
            .contents
            .iter()
            .any(|content| content.message_type() != message_type)
        {
            return Err(MessageError::MixedContentTypes);
        }

        Ok(message_type)
    }
}
