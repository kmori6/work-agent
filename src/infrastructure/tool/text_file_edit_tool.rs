use crate::domain::error::tool_error::ToolError;
use crate::domain::model::tool::ToolExecutionResult;
use crate::domain::port::tool::Tool;
use crate::infrastructure::util::path::{normalize_path, resolve_workspace_file_path};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::path::PathBuf;

const MAX_FILE_SIZE_BYTES: u64 = 1_048_576;

pub struct TextFileEditTool {
    workspace_root: PathBuf,
}

impl TextFileEditTool {
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

    fn line_start_offsets(content: &str) -> Vec<usize> {
        let mut offsets = Vec::new();
        let mut offset = 0usize;

        for segment in content.split_inclusive('\n') {
            offsets.push(offset);
            offset += segment.len();
        }

        if offsets.is_empty() && !content.is_empty() {
            offsets.push(0);
        }

        offsets
    }
}

#[async_trait]
impl Tool for TextFileEditTool {
    fn name(&self) -> &str {
        "text_file_edit"
    }

    fn description(&self) -> &str {
        "Edit a UTF-8 text file by replacing exact text, optionally limited to a line range."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the UTF-8 text file to edit. Relative paths are resolved from the workspace root. Absolute paths are allowed only if they stay inside the workspace."
                },
                "old_text": {
                    "type": "string",
                    "description": "Exact text to find and replace."
                },
                "new_text": {
                    "type": "string",
                    "description": "Replacement text. Use an empty string to delete the matched text."
                },
                "start_line": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Optional 1-based start line of the search scope."
                },
                "end_line": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Optional 1-based end line of the search scope."
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "Whether to replace all matches within the search scope. Default is false."
                }
            },
            "required": ["path", "old_text", "new_text"]
        })
    }

    async fn execute(&self, arguments: Value) -> Result<ToolExecutionResult, ToolError> {
        let path = arguments
            .get("path")
            .and_then(|value| value.as_str())
            .ok_or_else(|| ToolError::InvalidArguments("missing or invalid 'path'".into()))?;

        let old_text = arguments
            .get("old_text")
            .and_then(|value| value.as_str())
            .ok_or_else(|| ToolError::InvalidArguments("missing or invalid 'old_text'".into()))?;

        let new_text = arguments
            .get("new_text")
            .and_then(|value| value.as_str())
            .ok_or_else(|| ToolError::InvalidArguments("missing or invalid 'new_text'".into()))?;

        if old_text.is_empty() {
            return Err(ToolError::InvalidArguments(
                "'old_text' must not be empty".into(),
            ));
        }

        if old_text == new_text {
            return Err(ToolError::InvalidArguments(
                "'old_text' and 'new_text' must be different".into(),
            ));
        }

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

                Some(value)
            }
            None => None,
        };

        let end_line = match arguments.get("end_line") {
            Some(value) => {
                let value = value.as_u64().ok_or_else(|| {
                    ToolError::InvalidArguments("'end_line' must be an integer".into())
                })?;
                let value = usize::try_from(value).map_err(|_| {
                    ToolError::InvalidArguments("'end_line' is out of supported range".into())
                })?;

                if value == 0 {
                    return Err(ToolError::InvalidArguments(
                        "'end_line' must be greater than or equal to 1".into(),
                    ));
                }

                Some(value)
            }
            None => None,
        };

        if let (Some(start_line), Some(end_line)) = (start_line, end_line)
            && start_line > end_line
        {
            return Err(ToolError::InvalidArguments(
                "'start_line' must be less than or equal to 'end_line'".into(),
            ));
        }

        let replace_all = arguments
            .get("replace_all")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);

        let resolved_path = resolve_workspace_file_path(&self.workspace_root, path)?;

        let metadata = tokio::fs::metadata(&resolved_path).await.map_err(|err| {
            ToolError::ExecutionFailed(format!("failed to read file metadata: {err}"))
        })?;

        if metadata.len() > MAX_FILE_SIZE_BYTES {
            return Err(ToolError::ExecutionFailed(format!(
                "file is too large to edit safely: {} bytes (max: {MAX_FILE_SIZE_BYTES})",
                metadata.len()
            )));
        }

        let bytes = tokio::fs::read(&resolved_path)
            .await
            .map_err(|err| ToolError::ExecutionFailed(format!("failed to read file: {err}")))?;

        if bytes.contains(&0) {
            return Err(ToolError::ExecutionFailed(
                "file appears to be binary and cannot be edited with 'text_file_edit'".into(),
            ));
        }

        let content = String::from_utf8(bytes).map_err(|_| {
            ToolError::ExecutionFailed(
                "file is not valid UTF-8 text and cannot be edited with 'text_file_edit'".into(),
            )
        })?;

        let line_starts = Self::line_start_offsets(&content);
        let total_lines = line_starts.len();

        if let Some(start_line) = start_line
            && total_lines > 0
            && start_line > total_lines
        {
            return Err(ToolError::InvalidArguments(format!(
                "'start_line' is out of range for this file: {start_line} > {total_lines}"
            )));
        }

        if let Some(end_line) = end_line
            && total_lines > 0
            && end_line > total_lines
        {
            return Err(ToolError::InvalidArguments(format!(
                "'end_line' is out of range for this file: {end_line} > {total_lines}"
            )));
        }

        let effective_start_line = start_line.unwrap_or(1);
        let effective_end_line = end_line.unwrap_or(total_lines.max(1));

        let scope_start = if total_lines == 0 {
            0
        } else {
            line_starts[effective_start_line - 1]
        };

        let scope_end = if total_lines == 0 || effective_end_line >= total_lines {
            content.len()
        } else {
            line_starts[effective_end_line]
        };

        let scope = &content[scope_start..scope_end];
        let occurrence_count = scope.match_indices(old_text).count();

        if occurrence_count == 0 {
            return Err(ToolError::ExecutionFailed(
                "the specified 'old_text' was not found in the selected scope".into(),
            ));
        }

        if !replace_all && occurrence_count > 1 {
            return Err(ToolError::ExecutionFailed(format!(
                "the specified 'old_text' matched {occurrence_count} times in the selected scope; narrow the line range or set 'replace_all' to true"
            )));
        }

        // Edit only the requested line slice so repeated text outside the scope stays untouched.
        let replaced_scope = if replace_all {
            scope.replace(old_text, new_text)
        } else {
            scope.replacen(old_text, new_text, 1)
        };

        let mut updated_content =
            String::with_capacity(content.len().saturating_sub(scope.len()) + replaced_scope.len());
        updated_content.push_str(&content[..scope_start]);
        updated_content.push_str(&replaced_scope);
        updated_content.push_str(&content[scope_end..]);

        tokio::fs::write(&resolved_path, updated_content)
            .await
            .map_err(|err| ToolError::ExecutionFailed(format!("failed to write file: {err}")))?;

        let relative_path = resolved_path
            .strip_prefix(&self.workspace_root)
            .map(normalize_path)
            .unwrap_or_else(|_| normalize_path(&resolved_path));

        Ok(ToolExecutionResult::success(json!({
            "path": relative_path,
            "replaced_occurrences": if replace_all { occurrence_count } else { 1 },
            "scope": {
                "start_line": effective_start_line,
                "end_line": effective_end_line
            },
            "message": "Applied text edit successfully."
        })))
    }
}
