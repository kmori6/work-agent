use crate::domain::error::tool_error::ToolError;
use crate::domain::model::tool::ToolExecutionResult;
use crate::domain::port::tool::Tool;
use crate::infrastructure::util::path::{normalize_path, resolve_workspace_file_path};
use crate::infrastructure::util::text::truncate_text;
use async_trait::async_trait;
use aws_config::BehaviorVersion;
use aws_sdk_bedrockruntime::{
    Client,
    types::{
        CitationsConfig, ContentBlock, ConversationRole, DocumentBlock, DocumentFormat,
        DocumentSource, Message as BedrockMessage, SystemContentBlock,
    },
};
use aws_smithy_types::Blob;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const DEFAULT_MODEL: &str = "global.anthropic.claude-sonnet-4-6";
const DEFAULT_MAX_CHARS: usize = 50_000;
const MAX_OUTPUT_CHARS: usize = 200_000;
const MAX_PDF_BYTES: u64 = 100_000_000;
const PDF_TO_MARKDOWN_SYSTEM_PROMPT: &str = "\
You convert PDF documents into faithful Markdown transcriptions.
Return only Markdown and no surrounding commentary or code fences.
Preserve headings, paragraphs, lists, tables, captions, footnotes, and visible labels.
For charts, diagrams, figures, or other visual elements, describe the visible content in Markdown.
Do not summarize or omit sections.
If any content is unreadable, write [unclear].
";

pub struct PdfFileReadTool {
    workspace_root: PathBuf,
    model: String,
    default_max_chars: usize,
    max_chars_cap: usize,
}

impl PdfFileReadTool {
    pub fn new(workspace_root: impl Into<PathBuf>) -> Result<Self, ToolError> {
        let workspace_root = std::fs::canonicalize(workspace_root.into()).map_err(|err| {
            ToolError::Unavailable(format!("failed to resolve workspace root: {err}"))
        })?;

        if !workspace_root.is_dir() {
            return Err(ToolError::Unavailable(
                "workspace root must be a directory".into(),
            ));
        }

        let model = std::env::var("PDF_READ_TOOL_MODEL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_MODEL.to_string());

        Ok(Self {
            workspace_root,
            model,
            default_max_chars: DEFAULT_MAX_CHARS,
            max_chars_cap: MAX_OUTPUT_CHARS,
        })
    }

    async fn build_client(&self) -> Client {
        let config = aws_config::load_defaults(BehaviorVersion::latest()).await;
        Client::new(&config)
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
impl Tool for PdfFileReadTool {
    fn name(&self) -> &str {
        "pdf_file_read"
    }

    fn description(&self) -> &str {
        "Read a PDF (.pdf) file from the workspace and return its extracted Markdown text."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the .pdf file. Relative paths are resolved from the workspace root."
                },
                "max_chars": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": self.max_chars_cap,
                    "description": format!(
                        "Maximum number of characters to return. Default is {}. Maximum is {}.",
                        self.default_max_chars,
                        self.max_chars_cap
                    )
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

        let max_chars = self.parse_max_chars(&arguments)?;

        let resolved_path = resolve_workspace_file_path(&self.workspace_root, path)?;
        if !is_pdf_file(&resolved_path) {
            return Err(ToolError::InvalidArguments(
                "'path' must point to a .pdf file".into(),
            ));
        }

        let metadata = tokio::fs::metadata(&resolved_path).await.map_err(|err| {
            ToolError::ExecutionFailed(format!("failed to read file metadata: {err}"))
        })?;

        if metadata.len() > MAX_PDF_BYTES {
            return Err(ToolError::ExecutionFailed(format!(
                "PDF too large: {} bytes (limit: {MAX_PDF_BYTES} bytes)",
                metadata.len()
            )));
        }

        let bytes = tokio::fs::read(&resolved_path)
            .await
            .map_err(|err| ToolError::ExecutionFailed(format!("failed to read PDF file: {err}")))?;

        let citations = CitationsConfig::builder()
            .enabled(true)
            .build()
            .map_err(|err| {
                ToolError::ExecutionFailed(format!(
                    "failed to build Bedrock citations config: {err}"
                ))
            })?;

        let document = DocumentBlock::builder()
            .name("document")
            .format(DocumentFormat::Pdf)
            .source(DocumentSource::Bytes(Blob::new(bytes)))
            .citations(citations)
            .build()
            .map_err(|err| {
                ToolError::ExecutionFailed(format!("failed to build Bedrock document block: {err}"))
            })?;

        let message = BedrockMessage::builder()
            .role(ConversationRole::User)
            .content(ContentBlock::Text(
                "Convert the attached PDF into Markdown.".to_string(),
            ))
            .content(ContentBlock::Document(document))
            .build()
            .map_err(|err| {
                ToolError::ExecutionFailed(format!("failed to build Bedrock message: {err}"))
            })?;

        let client = self.build_client().await;
        let response = client
            .converse()
            .model_id(&self.model)
            .system(SystemContentBlock::Text(
                PDF_TO_MARKDOWN_SYSTEM_PROMPT.to_string(),
            ))
            .messages(message)
            .send()
            .await
            .map_err(|err| ToolError::ExecutionFailed(format!("Bedrock converse failed: {err}")))?;

        let output_blocks = response
            .output()
            .ok_or_else(|| ToolError::ExecutionFailed("no output in Bedrock response".into()))?
            .as_message()
            .map_err(|_| {
                ToolError::ExecutionFailed("unsupported output type in Bedrock response".into())
            })?
            .content();

        let markdown = extract_markdown(output_blocks);
        let markdown = if markdown.trim().is_empty() {
            "PDF contains no extractable Markdown text (it may be image-only, encrypted, or unsupported)."
                .to_string()
        } else {
            markdown
        };

        let relative_path = resolved_path
            .strip_prefix(&self.workspace_root)
            .map(normalize_path)
            .expect("resolved_path must be inside workspace_root");
        let (content, truncated) = truncate_text(markdown, max_chars);

        Ok(ToolExecutionResult::success(json!({
            "path": relative_path,
            "content": content,
            "truncated": truncated,
        })))
    }
}

fn is_pdf_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
}

fn extract_markdown(blocks: &[ContentBlock]) -> String {
    let mut parts = Vec::new();

    for block in blocks {
        if let Ok(text) = block.as_text() {
            let text = text.trim();
            if !text.is_empty() {
                parts.push(text.to_string());
            }
            continue;
        }

        if let Ok(citations_content) = block.as_citations_content() {
            let text = citations_content
                .content()
                .iter()
                .filter_map(|content| content.as_text().ok())
                .map(|text| text.as_str())
                .collect::<String>();

            let text = text.trim();
            if !text.is_empty() {
                parts.push(text.to_string());
            }
        }
    }

    parts.join("\n")
}
