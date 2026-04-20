use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentCliError {
    #[error("failed to initialize llm client: {0}")]
    LlmClient(#[from] crate::application::error::llm_client_error::LlmClientError),

    #[error("failed to execute agent use case: {0}")]
    Usecase(#[from] crate::application::error::agent_usecase_error::AgentUsecaseError),

    #[error("failed to execute research use case: {0}")]
    ResearchUsecase(
        #[from] crate::application::error::research_usecase_error::ResearchUsecaseError,
    ),

    #[error("failed to initialize tooling: {0}")]
    Tool(#[from] crate::domain::error::tool_error::ToolError),

    #[error("failed to initialize search provider: {0}")]
    Search(#[from] crate::domain::port::search_provider::SearchError),

    #[error("failed to read line input: {0}")]
    Readline(String),

    #[error("failed to read or write cli input/output: {0}")]
    Io(#[from] std::io::Error),

    #[error("failed to connect to database: {0}")]
    Database(#[from] sqlx::Error),
}
