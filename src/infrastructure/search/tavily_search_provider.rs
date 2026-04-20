use crate::domain::port::search_provider::{SearchDocument, SearchError, SearchProvider};
use async_trait::async_trait;
use std::time::Duration;
use tavily::{SearchRequest, Tavily, TavilyError};

const DEFAULT_TIMEOUT_SECS: u64 = 30;
const DEFAULT_MAX_RESULTS: i32 = 5;
const DEFAULT_TOPIC: &str = "general";
const DEFAULT_SEARCH_DEPTH: &str = "advanced";

pub struct TavilySearchProvider {
    api_key: String,
    client: Tavily,
    max_results: i32,
}

impl TavilySearchProvider {
    pub fn new(api_key: impl Into<String>) -> Result<Self, SearchError> {
        Self::with_config(api_key, DEFAULT_MAX_RESULTS)
    }

    pub fn with_config(api_key: impl Into<String>, max_results: i32) -> Result<Self, SearchError> {
        let api_key = api_key.into();
        let api_key = api_key.trim().to_string();

        if api_key.is_empty() {
            return Err(SearchError::Unavailable(
                "TAVILY_API_KEY must not be empty".to_string(),
            ));
        }

        if max_results <= 0 {
            return Err(SearchError::RequestFailed(
                "max_results must be greater than 0".to_string(),
            ));
        }

        let client = Tavily::builder(&api_key)
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .build()
            .map_err(map_tavily_error)?;

        Ok(Self {
            api_key,
            client,
            max_results,
        })
    }

    pub fn from_env() -> Result<Self, SearchError> {
        let api_key = std::env::var("TAVILY_API_KEY")
            .map_err(|_| SearchError::Unavailable("TAVILY_API_KEY is not set".to_string()))?;

        Self::new(api_key)
    }

    fn build_request(&self, query: &str) -> SearchRequest {
        SearchRequest::new(&self.api_key, query)
            .topic(DEFAULT_TOPIC)
            .search_depth(DEFAULT_SEARCH_DEPTH)
            .max_results(self.max_results)
            .include_raw_content(false)
            .include_answer(false)
    }
}

#[async_trait]
impl SearchProvider for TavilySearchProvider {
    async fn search(&self, query: &str) -> Result<Vec<SearchDocument>, SearchError> {
        let query = query.trim();
        if query.is_empty() {
            return Err(SearchError::RequestFailed(
                "search query must not be empty".to_string(),
            ));
        }

        let response = self
            .client
            .call(&self.build_request(query))
            .await
            .map_err(map_tavily_error)?;

        let documents = response
            .results
            .into_iter()
            .map(|item| SearchDocument {
                title: item.title,
                url: item.url,
                snippet: item.content,
            })
            .collect::<Vec<_>>();

        Ok(documents)
    }
}

fn map_tavily_error(err: TavilyError) -> SearchError {
    match err {
        TavilyError::Configuration(message) => {
            SearchError::Unavailable(format!("tavily configuration error: {message}"))
        }
        TavilyError::RateLimit(message) => {
            SearchError::Unavailable(format!("tavily rate limit: {message}"))
        }
        TavilyError::Http(err) if err.is_timeout() => {
            SearchError::RequestFailed("tavily request timed out".to_string())
        }
        TavilyError::Http(err) => SearchError::RequestFailed(format!("tavily http error: {err}")),
        TavilyError::Api(message) => {
            SearchError::ResponseParse(format!("tavily api error: {message}"))
        }
    }
}
