use async_trait::async_trait;
use thiserror::Error;

#[async_trait]
pub trait SearchProvider: Send + Sync {
    async fn search(&self, query: &str) -> Result<Vec<SearchDocument>, SearchError>;
}

#[derive(Debug, Clone)]
pub struct SearchDocument {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

#[derive(Debug, Error)]
pub enum SearchError {
    #[error("search provider is unavailable: {0}")]
    Unavailable(String),

    #[error("search request failed: {0}")]
    RequestFailed(String),

    #[error("search response could not be parsed: {0}")]
    ResponseParse(String),
}
