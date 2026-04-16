use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentCliError {
    #[error("failed to initialize llm client: {0}")]
    LlmClient(#[from] crate::application::error::llm_client_error::LlmClientError),

    #[error("failed to execute agent use case: {0}")]
    Usecase(#[from] crate::application::error::agent_usecase_error::AgentUsecaseError),

    #[error("failed to initialize tooling: {0}")]
    Tool(#[from] crate::domain::error::tool_error::ToolError),

    #[error("failed to read line input: {0}")]
    Readline(String),

    #[error("failed to read or write cli input/output: {0}")]
    Io(#[from] std::io::Error),
}
