use crate::domain::error::tool_error::ToolError;
use crate::domain::model::tool::ToolExecutionResult;
use crate::domain::port::tool::Tool;
use crate::infrastructure::util::path::{normalize_path, resolve_workspace_file_path};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::path::PathBuf;

const DEFAULT_LINE_COUNT: usize = 200;
const MAX_LINE_COUNT: usize = 400;
const MAX_FILE_SIZE_BYTES: u64 = 1_048_576;

pub struct TextFileReadTool {
    workspace_root: PathBuf,
}

impl TextFileReadTool {
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
}

#[async_trait]
impl Tool for TextFileReadTool {
    fn name(&self) -> &str {
        "text_file_read"
    }

    fn description(&self) -> &str {
        "Read a UTF-8 text file from the workspace with line numbers and optional line-range limits."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the text file to read. Relative paths are resolved from the workspace root. Absolute paths are allowed only if they stay inside the workspace."
                },
                "start_line": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "1-based line number to start reading from. Default is 1."
                },
                "line_count": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_LINE_COUNT,
                    "description": "Maximum number of lines to return. Default is 200."
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, arguments: Value) -> Result<ToolExecutionResult, ToolError> {
        let path = arguments
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArguments("missing or invalid 'path'".into()))?;

        let start_line = match arguments.get("start_line") {
            Some(value) => {
                let value = value.as_u64().ok_or_else(|| {
                    ToolError::InvalidArguments("'start_line' must be an integer".into())
                })?;
                let value = usize::try_from(value).map_err(|_| {
                    ToolError::InvalidArguments("'start_line' is out of supported range".into())
                })?;

                if value == 0 {
                    return Err(ToolError::InvalidArguments(
                        "'start_line' must be greater than or equal to 1".into(),
                    ));
                }

                value
            }
            None => 1,
        };

        let line_count = match arguments.get("line_count") {
            Some(value) => {
                let value = value.as_u64().ok_or_else(|| {
                    ToolError::InvalidArguments("'line_count' must be an integer".into())
                })?;
                let value = usize::try_from(value).map_err(|_| {
                    ToolError::InvalidArguments("'line_count' is out of supported range".into())
                })?;

                if value == 0 || value > MAX_LINE_COUNT {
                    return Err(ToolError::InvalidArguments(format!(
                        "'line_count' must be between 1 and {MAX_LINE_COUNT}"
                    )));
                }

                value
            }
            None => DEFAULT_LINE_COUNT,
        };

        let resolved_path = resolve_workspace_file_path(&self.workspace_root, path)?;

        let metadata = tokio::fs::metadata(&resolved_path).await.map_err(|err| {
            ToolError::ExecutionFailed(format!("failed to read file metadata: {err}"))
        })?;

        if metadata.len() > MAX_FILE_SIZE_BYTES {
            return Err(ToolError::ExecutionFailed(format!(
                "file is too large to read safely: {} bytes (max: {MAX_FILE_SIZE_BYTES})",
                metadata.len()
            )));
        }

        let bytes = tokio::fs::read(&resolved_path)
            .await
            .map_err(|err| ToolError::ExecutionFailed(format!("failed to read file: {err}")))?;

        if bytes.contains(&0) {
            return Err(ToolError::ExecutionFailed(
                "file appears to be binary and cannot be read with 'text_file_read'".into(),
            ));
        }

        let content = String::from_utf8(bytes).map_err(|_| {
            ToolError::ExecutionFailed(
                "file is not valid UTF-8 text and cannot be read with 'text_file_read'".into(),
            )
        })?;

        let lines: Vec<&str> = content.lines().collect();
        let total_lines = lines.len();
        let start_index = start_line.saturating_sub(1).min(total_lines);
        let end_index = start_index.saturating_add(line_count).min(total_lines);
        let visible_lines = &lines[start_index..end_index];

        // Include line numbers so the model can reference exact locations in follow-up calls.
        let formatted_content = visible_lines
            .iter()
            .enumerate()
            .map(|(index, line)| format!("{} | {}", start_index + index + 1, line))
            .collect::<Vec<_>>()
            .join("\n");

        let relative_path = resolved_path
            .strip_prefix(&self.workspace_root)
            .map(normalize_path)
            .unwrap_or_else(|_| normalize_path(&resolved_path));

        let returned_lines = visible_lines.len();
        let end_line = if returned_lines == 0 {
            start_line.saturating_sub(1)
        } else {
            start_index + returned_lines
        };

        Ok(ToolExecutionResult::success(json!({
            "path": relative_path,
            "start_line": start_line,
            "end_line": end_line,
            "total_lines": total_lines,
            "returned_lines": returned_lines,
            "truncated": end_index < total_lines,
            "content": formatted_content
        })))
    }
}
