use crate::application::error::llm_client_error::LlmClientError;
use crate::domain::port::search_provider::SearchError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DeepResearchError {
    #[error("llm request failed: {0}")]
    LlmClient(#[from] LlmClientError),

    #[error("search request failed: {0}")]
    Search(#[from] SearchError),
}
