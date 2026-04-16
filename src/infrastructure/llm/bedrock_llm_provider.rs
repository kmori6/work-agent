use crate::application::error::llm_client_error::LlmClientError;
use crate::domain::model::message::{Message, MessageContent};
use crate::domain::model::role::Role;
use crate::domain::model::tool::{ToolCall, ToolSpec};
use crate::domain::port::llm_provider::{LlmProvider, LlmResponse};
use async_trait::async_trait;
use aws_config::BehaviorVersion;
use aws_sdk_bedrockruntime::{
    Client,
    types::{
        ContentBlock, ConversationRole, Message as BedrockMessage, SystemContentBlock, Tool,
        ToolConfiguration, ToolInputSchema, ToolResultBlock, ToolResultContentBlock,
        ToolResultStatus, ToolSpecification, ToolUseBlock,
    },
};
use aws_smithy_types::{Document, Number};
use std::collections::HashMap;

pub struct BedrockLlmProvider {
    client: Client,
}

impl BedrockLlmProvider {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    pub async fn from_default_config() -> Self {
        let config = aws_config::load_defaults(BehaviorVersion::latest()).await;
        let client = Client::new(&config);
        Self::new(client)
    }

    async fn converse(
        &self,
        messages: Vec<Message>,
        model: &str,
        tools: Option<Vec<ToolSpec>>,
    ) -> Result<LlmResponse, LlmClientError> {
        let system_blocks = build_system_content_blocks(&messages)?;

        let message_blocks = build_content_block(&messages)?;

        let mut req = self
            .client
            .converse()
            .model_id(model)
            .set_messages(Some(message_blocks));

        for block in system_blocks {
            req = req.system(block);
        }

        if let Some(tools) = tools.as_ref().filter(|tools| !tools.is_empty()) {
            req = req.tool_config(tool_configuration(tools)?);
        }

        let output = req
            .send()
            .await
            .map_err(|e| LlmClientError::ApiCall(format!("Bedrock converse error: {}", e)))?;

        let output_blocks = output
            .output()
            .ok_or_else(|| {
                LlmClientError::ResponseParse("No output in Bedrock response".to_string())
            })?
            .as_message()
            .map_err(|_| {
                LlmClientError::ResponseParse(
                    "Unsupported output type in Bedrock response".to_string(),
                )
            })?
            .content();

        // LLM response
        let mut text_parts = Vec::new();
        let mut tool_calls = Vec::new();

        for block in output_blocks {
            if let Ok(text) = block.as_text() {
                text_parts.push(text.to_string());
                continue;
            }

            if let Ok(tool_use) = block.as_tool_use() {
                tool_calls.push(ToolCall {
                    id: tool_use.tool_use_id().to_string(),
                    name: tool_use.name().to_string(),
                    arguments: document_to_json(tool_use.input())?,
                });
            }
        }

        Ok(LlmResponse {
            text: text_parts.join("\n"),
            tool_calls,
        })
    }
}

#[async_trait]
impl LlmProvider for BedrockLlmProvider {
    async fn response(
        &self,
        messages: Vec<Message>,
        model: &str,
    ) -> Result<String, LlmClientError> {
        Ok(self.converse(messages, model, None).await?.text)
    }

    async fn response_with_tool(
        &self,
        messages: Vec<Message>,
        tools: Vec<ToolSpec>,
        model: &str,
    ) -> Result<LlmResponse, LlmClientError> {
        self.converse(messages, model, Some(tools)).await
    }
}

/// Converts system messages to Bedrock SystemContentBlocks.
fn build_system_content_blocks(
    messages: &[Message],
) -> Result<Vec<SystemContentBlock>, LlmClientError> {
    messages
        .iter()
        .filter(|m| m.role == Role::System)
        .map(|m| match &m.content {
            MessageContent::Text(text) => Ok(SystemContentBlock::Text(text.clone())),
            MessageContent::ToolCall { .. } => Err(LlmClientError::RequestBuild(
                "System messages cannot contain tool calls".to_string(),
            )),
            MessageContent::ToolResult(_) => Err(LlmClientError::RequestBuild(
                "System messages cannot contain tool results".to_string(),
            )),
        })
        .collect()
}

/// Converts domain messages to Bedrock messages.
fn build_content_block(messages: &[Message]) -> Result<Vec<BedrockMessage>, LlmClientError> {
    let mut message_blocks: Vec<BedrockMessage> = vec![];
    for m in messages.iter().filter(|m| m.role != Role::System) {
        let role = match m.role {
            Role::Assistant => ConversationRole::Assistant,
            _ => ConversationRole::User,
        };
        let msg = match &m.content {
            MessageContent::Text(text) => BedrockMessage::builder()
                .role(role.clone())
                .content(ContentBlock::Text(text.clone()))
                .build()
                .map_err(|e| {
                    LlmClientError::RequestBuild(format!("Error building Bedrock message: {}", e))
                })?,
            MessageContent::ToolCall { text, tool_calls } => {
                let mut builder = BedrockMessage::builder().role(ConversationRole::Assistant);

                if let Some(text) = text.as_ref().filter(|text| !text.is_empty()) {
                    builder = builder.content(ContentBlock::Text(text.clone()));
                }

                for tool_call in tool_calls {
                    let tool_use = ToolUseBlock::builder()
                        .tool_use_id(tool_call.id.clone())
                        .name(tool_call.name.clone())
                        .input(json_to_document(&tool_call.arguments)?)
                        .build()
                        .map_err(|e| {
                            LlmClientError::RequestBuild(format!(
                                "Error building Bedrock tool use block: {}",
                                e
                            ))
                        })?;

                    builder = builder.content(ContentBlock::ToolUse(tool_use));
                }

                builder.build().map_err(|e| {
                    LlmClientError::RequestBuild(format!("Error building Bedrock message: {}", e))
                })?
            }
            MessageContent::ToolResult(tool_result) => {
                let result_content = ToolResultContentBlock::Text(
                    serde_json::to_string(&tool_result.output).map_err(|e| {
                        LlmClientError::RequestBuild(format!(
                            "Failed to serialize tool result output: {}",
                            e
                        ))
                    })?,
                );

                let status = if tool_result.is_error {
                    ToolResultStatus::Error
                } else {
                    ToolResultStatus::Success
                };

                let block = ToolResultBlock::builder()
                    .tool_use_id(tool_result.tool_call_id.clone())
                    .content(result_content)
                    .status(status)
                    .build()
                    .map_err(|e| {
                        LlmClientError::RequestBuild(format!(
                            "Error building Bedrock tool result block: {}",
                            e
                        ))
                    })?;

                BedrockMessage::builder()
                    .role(role.clone())
                    .content(ContentBlock::ToolResult(block))
                    .build()
                    .map_err(|e| {
                        LlmClientError::RequestBuild(format!(
                            "Error building Bedrock message: {}",
                            e
                        ))
                    })?
            }
        };
        message_blocks.push(msg);
    }

    Ok(message_blocks)
}

/// Converts a Vec of ToolCall to a Bedrock ToolConfiguration.
fn tool_configuration(tools: &[ToolSpec]) -> Result<ToolConfiguration, LlmClientError> {
    let mut builder = ToolConfiguration::builder();

    for tool in tools {
        let spec = ToolSpecification::builder()
            .name(tool.name.clone())
            .description(tool.description.clone())
            .input_schema(ToolInputSchema::Json(json_to_document(&tool.parameters)?))
            .build()
            .map_err(|e| {
                LlmClientError::RequestBuild(format!(
                    "Error building Bedrock tool specification: {}",
                    e
                ))
            })?;

        builder = builder.tools(Tool::ToolSpec(spec));
    }

    builder.build().map_err(|e| {
        LlmClientError::RequestBuild(format!("Error building Bedrock tool configuration: {}", e))
    })
}

/// Converts a Bedrock Document to a serde_json::Value.
fn document_to_json(document: &Document) -> Result<serde_json::Value, LlmClientError> {
    match document {
        Document::Object(object) => {
            let mut map = serde_json::Map::new();
            for (key, value) in object {
                map.insert(key.clone(), document_to_json(value)?);
            }
            Ok(serde_json::Value::Object(map))
        }
        Document::Array(array) => Ok(serde_json::Value::Array(
            array
                .iter()
                .map(document_to_json)
                .collect::<Result<Vec<_>, _>>()?,
        )),
        Document::Number(number) => match number {
            Number::PosInt(value) => Ok(serde_json::Value::Number((*value).into())),
            Number::NegInt(value) => Ok(serde_json::Value::Number((*value).into())),
            Number::Float(value) => serde_json::Number::from_f64(*value)
                .map(serde_json::Value::Number)
                .ok_or_else(|| {
                    LlmClientError::ResponseParse(format!(
                        "Bedrock tool input contains non-finite float: {}",
                        value
                    ))
                }),
        },
        Document::String(value) => Ok(serde_json::Value::String(value.clone())),
        Document::Bool(value) => Ok(serde_json::Value::Bool(*value)),
        Document::Null => Ok(serde_json::Value::Null),
    }
}

/// Converts a serde_json::Value to a Bedrock Document.
fn json_to_document(value: &serde_json::Value) -> Result<Document, LlmClientError> {
    match value {
        serde_json::Value::Object(object) => {
            let mut map = HashMap::new();
            for (key, value) in object {
                map.insert(key.clone(), json_to_document(value)?);
            }
            Ok(Document::Object(map))
        }
        serde_json::Value::Array(array) => Ok(Document::Array(
            array
                .iter()
                .map(json_to_document)
                .collect::<Result<Vec<_>, _>>()?,
        )),
        serde_json::Value::Number(number) => {
            if let Some(value) = number.as_u64() {
                Ok(Document::Number(Number::PosInt(value)))
            } else if let Some(value) = number.as_i64() {
                if value < 0 {
                    Ok(Document::Number(Number::NegInt(value)))
                } else {
                    Ok(Document::Number(Number::PosInt(value as u64)))
                }
            } else if let Some(value) = number.as_f64() {
                Ok(Document::Number(Number::Float(value)))
            } else {
                Err(LlmClientError::RequestBuild(format!(
                    "Unsupported JSON number for Bedrock tool schema: {}",
                    number
                )))
            }
        }
        serde_json::Value::String(value) => Ok(Document::String(value.clone())),
        serde_json::Value::Bool(value) => Ok(Document::Bool(*value)),
        serde_json::Value::Null => Ok(Document::Null),
    }
}
