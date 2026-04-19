use crate::domain::error::tool_error::ToolError;
use crate::domain::model::tool::ToolExecutionResult;
use crate::domain::port::tool::Tool;
use crate::infrastructure::util::path::resolve_workspace_file_path;
use async_trait::async_trait;
use aws_config::BehaviorVersion;
use aws_sdk_bedrockruntime::primitives::Blob;
use aws_sdk_bedrockruntime::types::{
    ContentBlock, ConversationRole, DocumentBlock, DocumentFormat, DocumentSource, ImageBlock,
    ImageFormat, ImageSource, Message,
};
use serde_json::{Value, json};
use std::path::Path;
use std::path::PathBuf;

const MODEL: &str = "global.anthropic.claude-sonnet-4-6";

pub struct OcrTool {
    workspace_root: PathBuf,
}

impl OcrTool {
    pub fn new(workspace_root: impl Into<PathBuf>) -> Result<Self, ToolError> {
        let workspace_root = std::fs::canonicalize(workspace_root.into()).map_err(|err| {
            ToolError::Unavailable(format!("failed to resolve workspace root: {err}"))
        })?;

        if !workspace_root.is_dir() {
            return Err(ToolError::Unavailable(
                "workspace root must be a directory".into(),
            ));
        }

        Ok(Self { workspace_root })
    }

    pub fn output_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "Extracted UTF-8 text."
                }
            },
            "required": ["content"]
        })
    }
}

#[async_trait]
impl Tool for OcrTool {
    fn name(&self) -> &str {
        "ocr"
    }

    fn description(&self) -> &str {
        "Extract text from a local image or PDF file."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the local image or PDF file to OCR. Relative paths are resolved from the workspace root. Absolute paths are allowed only if they stay inside the workspace."
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

        let resolved_path = resolve_workspace_file_path(&self.workspace_root, path)?;
        let file_bytes = tokio::fs::read(&resolved_path).await.map_err(|err| {
            ToolError::ExecutionFailed(format!("failed to read OCR source file: {err}"))
        })?;

        if file_bytes.is_empty() {
            return Err(ToolError::ExecutionFailed(
                "OCR source file is empty".into(),
            ));
        }

        // bedrock setup
        let config = aws_config::load_defaults(BehaviorVersion::latest()).await;
        let client = aws_sdk_bedrockruntime::Client::new(&config);
        let prompt = "Extract all visible text verbatim in reading order. Return plain text only.";

        let user_message = if is_pdf(&resolved_path) {
            Message::builder()
                .role(ConversationRole::User)
                .content(ContentBlock::Text(prompt.to_string()))
                .content(ContentBlock::Document(
                    DocumentBlock::builder()
                        .name("document")
                        .format(DocumentFormat::Pdf)
                        .source(DocumentSource::Bytes(Blob::new(file_bytes)))
                        .build()
                        .map_err(|err| {
                            ToolError::ExecutionFailed(format!(
                                "failed to build document block: {err}"
                            ))
                        })?,
                ))
                .build()
                .map_err(|err| {
                    ToolError::ExecutionFailed(format!("failed to build message: {err}"))
                })?
        } else {
            let image_format = infer_image_format(&resolved_path)?;
            Message::builder()
                .role(ConversationRole::User)
                .content(ContentBlock::Text(prompt.to_string()))
                .content(ContentBlock::Image(
                    ImageBlock::builder()
                        .format(image_format)
                        .source(ImageSource::Bytes(Blob::new(file_bytes)))
                        .build()
                        .map_err(|err| {
                            ToolError::ExecutionFailed(format!(
                                "failed to build image block: {err}"
                            ))
                        })?,
                ))
                .build()
                .map_err(|err| {
                    ToolError::ExecutionFailed(format!("failed to build message: {err}"))
                })?
        };

        let response = client
            .converse()
            .model_id(MODEL)
            .messages(user_message)
            .send()
            .await
            .map_err(|err| {
                let message = err.to_string();
                if message.to_ascii_lowercase().contains("timeout") {
                    ToolError::Timeout
                } else {
                    ToolError::ExecutionFailed(format!("bedrock converse failed: {message}"))
                }
            })?;

        let content = response
            .output()
            .and_then(|o| o.as_message().ok())
            .map(|m| {
                m.content()
                    .iter()
                    .filter_map(|b| b.as_text().ok().map(|s| s.as_str()))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();

        if content.trim().is_empty() {
            return Err(ToolError::ExecutionFailed(
                "bedrock returned empty OCR result".into(),
            ));
        }

        Ok(ToolExecutionResult::success(json!({
            "content": content
        })))
    }
}

fn infer_image_format(path: &Path) -> Result<ImageFormat, ToolError> {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => Ok(ImageFormat::Png),
        Some("jpg") | Some("jpeg") => Ok(ImageFormat::Jpeg),
        Some("gif") => Ok(ImageFormat::Gif),
        Some("webp") => Ok(ImageFormat::Webp),
        Some(other) => Err(ToolError::InvalidArguments(format!(
            "unsupported image format for OCR: {other}"
        ))),
        None => Err(ToolError::InvalidArguments(
            "file extension is required".into(),
        )),
    }
}

fn is_pdf(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("pdf")),
        Some(true)
    )
}
