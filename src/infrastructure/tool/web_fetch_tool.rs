use crate::domain::error::tool_error::ToolError;
use crate::domain::model::tool::ToolExecutionResult;
use crate::domain::port::tool::Tool;
use crate::infrastructure::util::text::truncate_text;
use crate::infrastructure::util::url::validate_external_url;
use async_trait::async_trait;
use reqwest::{header, redirect::Policy};
use serde_json::{Value, json};
use std::time::Duration;

const DEFAULT_MAX_CHARS: usize = 12_000;
const MAX_CHARS: usize = 50_000;
const DEFAULT_TIMEOUT_SECS: u64 = 20;
const DEFAULT_USER_AGENT: &str = "work-agent/0.1 (web_fetch)";

pub struct WebFetchTool {
    client: reqwest::Client,
    default_max_chars: usize,
    max_chars_cap: usize,
}

impl WebFetchTool {
    pub fn new() -> Result<Self, ToolError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .redirect(Policy::limited(5))
            .user_agent(DEFAULT_USER_AGENT)
            .build()
            .map_err(|err| ToolError::Unavailable(format!("failed to build HTTP client: {err}")))?;

        Ok(Self {
            client,
            default_max_chars: DEFAULT_MAX_CHARS,
            max_chars_cap: MAX_CHARS,
        })
    }

    fn parse_max_chars(&self, arguments: &Value) -> Result<usize, ToolError> {
        match arguments.get("max_chars") {
            Some(value) => {
                let value = value.as_u64().ok_or_else(|| {
                    ToolError::InvalidArguments("'max_chars' must be an integer".into())
                })?;
                let value = usize::try_from(value).map_err(|_| {
                    ToolError::InvalidArguments("'max_chars' is out of supported range".into())
                })?;
                if value == 0 || value > self.max_chars_cap {
                    return Err(ToolError::InvalidArguments(format!(
                        "'max_chars' must be between 1 and {}",
                        self.max_chars_cap
                    )));
                }
                Ok(value)
            }
            None => Ok(self.default_max_chars),
        }
    }
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetch a web page by URL and return readable text content. HTML is converted to plain text automatically."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "HTTP or HTTPS URL to fetch."
                },
                "max_chars": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": self.max_chars_cap,
                    "description": "Maximum number of characters to return. Default is 12000."
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, arguments: Value) -> Result<ToolExecutionResult, ToolError> {
        let raw_url = arguments
            .get("url")
            .and_then(|value| value.as_str())
            .ok_or_else(|| ToolError::InvalidArguments("missing or invalid 'url'".into()))?;

        let max_chars = self.parse_max_chars(&arguments)?;
        let url = validate_external_url(raw_url)?;

        let response = self.client.get(url.clone()).send().await.map_err(|err| {
            if err.is_timeout() {
                ToolError::Timeout
            } else {
                ToolError::Unavailable(format!("failed to fetch URL: {err}"))
            }
        })?;

        if !response.status().is_success() {
            return Err(ToolError::ExecutionFailed(format!(
                "HTTP {}",
                response.status().as_u16()
            )));
        }

        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_lowercase();

        let body = response.text().await.map_err(|err| {
            ToolError::ExecutionFailed(format!("failed to read response body: {err}"))
        })?;

        let text = extract_text_from_body(&content_type, &body)?;
        let (content, truncated) = truncate_text(text, max_chars);

        Ok(ToolExecutionResult::success(json!({
            "content": content,
            "truncated": truncated,
        })))
    }
}

fn extract_text_from_body(content_type: &str, body: &str) -> Result<String, ToolError> {
    let is_html = content_type.is_empty() || content_type.contains("text/html");
    let is_json = content_type.contains("application/json") || content_type.contains("+json");
    let is_text = content_type.starts_with("text/")
        || content_type.contains("application/xml")
        || content_type.contains("application/javascript");

    if is_html {
        let text = html2text::from_read(body.as_bytes(), 80)
            .map_err(|err| ToolError::ExecutionFailed(format!("failed to convert HTML: {err}")))?;
        Ok(text.trim().to_string())
    } else if is_json {
        let pretty = serde_json::from_str::<serde_json::Value>(body)
            .ok()
            .and_then(|v| serde_json::to_string_pretty(&v).ok())
            .unwrap_or_else(|| body.to_string());
        Ok(pretty)
    } else if is_text {
        Ok(body.to_string())
    } else {
        Err(ToolError::ExecutionFailed(format!(
            "unsupported content type: {content_type}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fetches_example_dot_com() {
        let tool = WebFetchTool::new().unwrap();
        let result = tool
            .execute(json!({ "url": "https://example.com" }))
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(
            result.output["content"]
                .as_str()
                .unwrap()
                .contains("Example Domain")
        );
    }
}
