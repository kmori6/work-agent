use crate::application::error::llm_client_error::LlmClientError;
use crate::domain::model::message::Message;
use crate::domain::model::tool::{ToolCall, ToolSpec};
use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn response(&self, messages: Vec<Message>, model: &str)
    -> Result<String, LlmClientError>;

    async fn response_with_tool(
        &self,
        messages: Vec<Message>,
        tools: Vec<ToolSpec>,
        model: &str,
    ) -> Result<LlmResponse, LlmClientError>;
}
