use crate::domain::error::tool_error::ToolError;
use crate::domain::model::tool::ToolExecutionResult;
use crate::domain::port::tool::Tool;
use crate::infrastructure::util::path::{normalize_path, resolve_workspace_file_path};
use async_trait::async_trait;
use reqwest::{Url, multipart};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;

const DEFAULT_ASR_BASE_URL: &str = "http://localhost:8000";
const DEFAULT_TIMEOUT_SECS: u64 = 300;

pub struct AsrTool {
    workspace_root: PathBuf,
    client: reqwest::Client,
    base_url: Url,
    api_key: Option<String>,
}

impl AsrTool {
    pub fn new(
        workspace_root: impl Into<PathBuf>,
        base_url: impl AsRef<str>,
        timeout_secs: u64,
        api_key: Option<String>,
    ) -> Result<Self, ToolError> {
        let workspace_root = std::fs::canonicalize(workspace_root.into()).map_err(|err| {
            ToolError::Unavailable(format!("failed to resolve workspace root: {err}"))
        })?;

        if !workspace_root.is_dir() {
            return Err(ToolError::Unavailable(
                "workspace root must be a directory".into(),
            ));
        }

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .map_err(|err| ToolError::Unavailable(format!("failed to build HTTP client: {err}")))?;

        let raw_base_url = base_url.as_ref().trim();
        if raw_base_url.is_empty() {
            return Err(ToolError::Unavailable(
                "ASR base URL must not be empty".into(),
            ));
        }

        let normalized_base_url = raw_base_url.trim_end_matches('/');
        let base_url = Url::parse(normalized_base_url)
            .map_err(|err| ToolError::Unavailable(format!("invalid ASR base URL: {err}")))?;

        let api_key = api_key.and_then(|key| {
            let trimmed = key.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });

        Ok(Self {
            workspace_root,
            client,
            base_url,
            api_key,
        })
    }

    pub fn from_env(workspace_root: impl Into<PathBuf>) -> Result<Self, ToolError> {
        let base_url = std::env::var("ASR_BASE_URL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_ASR_BASE_URL.to_string());
        let timeout_secs = std::env::var("ASR_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(DEFAULT_TIMEOUT_SECS);
        let api_key = std::env::var("ASR_API_KEY").ok();

        Self::new(workspace_root, base_url, timeout_secs, api_key)
    }

    fn parse_language(arguments: &Value) -> Result<Option<String>, ToolError> {
        match arguments.get("language") {
            Some(value) => {
                let language = value.as_str().ok_or_else(|| {
                    ToolError::InvalidArguments("'language' must be a string".into())
                })?;
                let language = language.trim();
                if language.is_empty() {
                    return Err(ToolError::InvalidArguments(
                        "'language' must not be empty".into(),
                    ));
                }
                Ok(Some(language.to_string()))
            }
            None => Ok(None),
        }
    }

    fn infer_mime_type(path: &Path) -> &'static str {
        match path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| extension.to_ascii_lowercase())
            .as_deref()
        {
            Some("mp3") => "audio/mpeg",
            Some("mp4") | Some("m4a") => "video/mp4",
            Some("wav") => "audio/wav",
            Some("webm") => "video/webm",
            Some("ogg") => "audio/ogg",
            Some("flac") => "audio/flac",
            _ => "application/octet-stream",
        }
    }
}

#[async_trait]
impl Tool for AsrTool {
    fn name(&self) -> &str {
        "asr"
    }

    fn description(&self) -> &str {
        "Transcribe a local audio or video file using a configured OpenAI-compatible ASR endpoint."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the local audio or video file to transcribe. Relative paths are resolved from the workspace root. Absolute paths are allowed only if they stay inside the workspace."
                },
                "language": {
                    "type": "string",
                    "description": "Optional language code such as 'ja' or 'en'. Use this when the spoken language is known."
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, arguments: Value) -> Result<ToolExecutionResult, ToolError> {
        let path = arguments
            .get("path")
            .and_then(|value| value.as_str())
            .ok_or_else(|| ToolError::InvalidArguments("missing or invalid 'path'".into()))?;

        let language = Self::parse_language(&arguments)?;
        let resolved_path = resolve_workspace_file_path(&self.workspace_root, path)?;
        let file_name = resolved_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                ToolError::InvalidArguments("'path' must include a valid file name".into())
            })?
            .to_string();

        let file_bytes = tokio::fs::read(&resolved_path).await.map_err(|err| {
            ToolError::ExecutionFailed(format!("failed to read media file: {err}"))
        })?;

        let file_part = multipart::Part::bytes(file_bytes)
            .file_name(file_name)
            .mime_str(Self::infer_mime_type(&resolved_path))
            .map_err(|err| {
                ToolError::ExecutionFailed(format!("failed to build upload part: {err}"))
            })?;

        let mut form = multipart::Form::new()
            .part("file", file_part)
            .text("response_format", "verbose_json");

        if let Some(language) = language {
            form = form.text("language", language);
        }

        let endpoint = self
            .base_url
            .join("v1/audio/transcriptions")
            .map_err(|err| {
                ToolError::Unavailable(format!("invalid ASR endpoint configuration: {err}"))
            })?;

        let mut request = self.client.post(endpoint).multipart(form);
        if let Some(api_key) = &self.api_key {
            request = request.bearer_auth(api_key);
        }

        let response = request.send().await.map_err(|err| {
            if err.is_timeout() {
                ToolError::Timeout
            } else {
                ToolError::Unavailable(format!("failed to call ASR endpoint: {err}"))
            }
        })?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            let detail = body.trim();

            let message = if detail.is_empty() {
                format!("ASR endpoint returned HTTP {status}")
            } else {
                format!("ASR endpoint returned HTTP {status}: {detail}")
            };

            return Err(ToolError::ExecutionFailed(message));
        }

        let body = response.text().await.map_err(|err| {
            ToolError::ExecutionFailed(format!("failed to read ASR response body: {err}"))
        })?;

        let response_json: Value = serde_json::from_str(&body).map_err(|err| {
            ToolError::ExecutionFailed(format!("failed to parse ASR response JSON: {err}"))
        })?;

        let segments = response_json.get("segments").and_then(|v| v.as_array());

        let transcript = match segments {
            Some(segments) => segments
                .iter()
                .filter_map(|seg| {
                    let start = seg.get("start")?.as_f64()?;
                    let text = seg.get("text")?.as_str()?;
                    let mins = (start / 60.0) as u32;
                    let secs = start % 60.0;
                    Some(format!("[{mins:02}:{secs:05.2}] {}", text.trim()))
                })
                .collect::<Vec<_>>()
                .join("\n"),
            None => response_json
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        };

        let relative_path = resolved_path
            .strip_prefix(&self.workspace_root)
            .map(normalize_path)
            .expect("resolved_path must be inside workspace_root");

        Ok(ToolExecutionResult::success(json!({
            "path": relative_path,
            "transcript": transcript
        })))
    }
}
