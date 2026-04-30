use crate::domain::error::llm_provider_error::LlmProviderError;
use crate::domain::model::message::Message;
use crate::domain::model::token_usage::TokenUsage;
use crate::domain::model::tool_call::{ToolCall, ToolSpec};
use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: TokenUsage,
}

#[derive(Debug, Clone)]
pub struct StructuredOutputSchema {
    pub name: String,
    pub description: Option<String>,
    pub schema: serde_json::Value,
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn response(
        &self,
        messages: Vec<Message>,
        model: &str,
    ) -> Result<String, LlmProviderError>;

    async fn response_with_tool(
        &self,
        messages: Vec<Message>,
        tools: Vec<ToolSpec>,
        model: &str,
    ) -> Result<LlmResponse, LlmProviderError>;

    async fn response_with_structure(
        &self,
        messages: Vec<Message>,
        schema: StructuredOutputSchema,
        model: &str,
    ) -> Result<serde_json::Value, LlmProviderError>;
}
