use crate::domain::error::tool_error::ToolError;
use crate::domain::model::tool::ToolExecutionResult;
use crate::domain::port::tool::Tool;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::time::Duration;
use tavily::{SearchRequest, Tavily, TavilyError};

const DURATION_SECS: u64 = 30;

pub struct WebSearchTool {
    api_key: String,
    client: Tavily,
}

impl WebSearchTool {
    pub fn new(api_key: impl Into<String>) -> Result<Self, ToolError> {
        let api_key = api_key.into();

        let client = Tavily::builder(&api_key)
            .timeout(Duration::from_secs(DURATION_SECS))
            .build()
            .map_err(map_tavily_error)?;

        Ok(Self { api_key, client })
    }

    pub fn from_env() -> Result<Self, ToolError> {
        let api_key = std::env::var("TAVILY_API_KEY")
            .map_err(|_| ToolError::Unavailable("TAVILY_API_KEY is not set".into()))?;

        Self::new(api_key)
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the web using Tavily. Supports general and news search."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query"
                },
                "topic": {
                    "type": "string",
                    "enum": ["general", "news"],
                    "description": "Search topic"
                },
                "depth": {
                    "type": "string",
                    "enum": ["basic", "advanced"],
                    "description": "Search depth"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of results"
                },
                "days": {
                    "type": "integer",
                    "description": "Only used for news topic"
                },
                "include_raw_content": {
                    "type": "boolean"
                },
                "include_answer": {
                    "type": "boolean"
                },
                "include_domains": {
                    "type": "array",
                    "items": { "type": "string" }
                },
                "exclude_domains": {
                    "type": "array",
                    "items": { "type": "string" }
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, arguments: Value) -> Result<ToolExecutionResult, ToolError> {
        let query = arguments
            .get("query")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|query| !query.is_empty())
            .ok_or_else(|| ToolError::InvalidArguments("missing or invalid 'query'".into()))?;

        let topic = match arguments.get("topic") {
            Some(value) => {
                let topic = value
                    .as_str()
                    .ok_or_else(|| ToolError::InvalidArguments("'topic' must be a string".into()))?
                    .to_lowercase();

                if topic != "general" && topic != "news" {
                    return Err(ToolError::InvalidArguments(
                        "'topic' must be one of: general, news".into(),
                    ));
                }

                topic
            }
            None => "general".to_string(),
        };

        let depth = match arguments.get("depth") {
            Some(value) => {
                let depth = value
                    .as_str()
                    .ok_or_else(|| ToolError::InvalidArguments("'depth' must be a string".into()))?
                    .to_lowercase();

                if depth != "basic" && depth != "advanced" {
                    return Err(ToolError::InvalidArguments(
                        "'depth' must be one of: basic, advanced".into(),
                    ));
                }

                depth
            }
            None => "basic".to_string(),
        };

        let max_results = match arguments.get("max_results") {
            Some(value) => {
                let max_results = value.as_i64().ok_or_else(|| {
                    ToolError::InvalidArguments("'max_results' must be an integer".into())
                })?;

                i32::try_from(max_results).map_err(|_| {
                    ToolError::InvalidArguments("'max_results' is out of supported range".into())
                })?
            }
            None => 5,
        };

        if max_results <= 0 {
            return Err(ToolError::InvalidArguments(
                "'max_results' must be greater than 0".into(),
            ));
        }

        let days = match arguments.get("days") {
            Some(value) => {
                let days = value.as_i64().ok_or_else(|| {
                    ToolError::InvalidArguments("'days' must be an integer".into())
                })?;

                Some(i32::try_from(days).map_err(|_| {
                    ToolError::InvalidArguments("'days' is out of supported range".into())
                })?)
            }
            None => None,
        };

        let include_raw_content = arguments
            .get("include_raw_content")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let include_answer = arguments
            .get("include_answer")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let include_domains = match arguments.get("include_domains") {
            Some(value) => {
                let domains = value.as_array().ok_or_else(|| {
                    ToolError::InvalidArguments("'include_domains' must be an array".into())
                })?;

                let mut include_domains = Vec::with_capacity(domains.len());
                for (index, domain) in domains.iter().enumerate() {
                    include_domains.push(
                        domain
                            .as_str()
                            .ok_or_else(|| {
                                ToolError::InvalidArguments(format!(
                                    "'include_domains[{index}]' must be a string"
                                ))
                            })?
                            .to_string(),
                    );
                }

                include_domains
            }
            None => Vec::new(),
        };

        let exclude_domains = match arguments.get("exclude_domains") {
            Some(value) => {
                let domains = value.as_array().ok_or_else(|| {
                    ToolError::InvalidArguments("'exclude_domains' must be an array".into())
                })?;

                let mut exclude_domains = Vec::with_capacity(domains.len());
                for (index, domain) in domains.iter().enumerate() {
                    exclude_domains.push(
                        domain
                            .as_str()
                            .ok_or_else(|| {
                                ToolError::InvalidArguments(format!(
                                    "'exclude_domains[{index}]' must be a string"
                                ))
                            })?
                            .to_string(),
                    );
                }

                exclude_domains
            }
            None => Vec::new(),
        };

        let mut request = SearchRequest::new(&self.api_key, query)
            .topic(&topic)
            .search_depth(&depth)
            .max_results(max_results)
            .include_raw_content(include_raw_content)
            .include_answer(include_answer);

        if topic == "news"
            && let Some(days) = days
        {
            request = request.days(days);
        }

        if !include_domains.is_empty() {
            request = request.include_domains(include_domains);
        }

        if !exclude_domains.is_empty() {
            request = request.exclude_domains(exclude_domains);
        }

        let response = self.client.call(&request).await.map_err(map_tavily_error)?;

        Ok(ToolExecutionResult::success(json!({
            "query": response.query,
            "answer": response.answer,
            "response_time": response.response_time,
            "follow_up_questions": response.follow_up_questions,
            "images": response.images,
            "results": response.results.into_iter().map(|item| json!({
                "title": item.title,
                "url": item.url,
                "content": item.content,
                "score": item.score,
                "raw_content": item.raw_content
            })).collect::<Vec<_>>()
        })))
    }
}

fn map_tavily_error(err: TavilyError) -> ToolError {
    match err {
        TavilyError::Configuration(msg) => {
            ToolError::Unavailable(format!("tavily configuration error: {msg}"))
        }
        TavilyError::RateLimit(msg) => ToolError::Unavailable(format!("tavily rate limit: {msg}")),
        TavilyError::Http(err) if err.is_timeout() => ToolError::Timeout,
        TavilyError::Http(err) => ToolError::Unavailable(format!("tavily http error: {err}")),
        TavilyError::Api(msg) => ToolError::ExecutionFailed(format!("tavily api error: {msg}")),
    }
}
