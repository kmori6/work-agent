use crate::domain::error::tool_error::ToolError;
use crate::domain::model::tool::ToolExecutionResult;
use crate::domain::port::tool::Tool;
use crate::infrastructure::util::path::{normalize_path, read_workspace_text_file};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::path::PathBuf;

pub struct TextFileReadTool {
    workspace_root: PathBuf,
    max_file_size: u64,
    max_line_count: usize,
}

impl TextFileReadTool {
    pub fn new(
        workspace_root: impl Into<PathBuf>,
        max_file_size: u64,
        max_line_count: usize,
    ) -> Result<Self, ToolError> {
        let workspace_root = std::fs::canonicalize(workspace_root.into()).map_err(|err| {
            ToolError::Unavailable(format!("failed to resolve workspace root: {err}"))
        })?;

        if !workspace_root.is_dir() {
            return Err(ToolError::Unavailable(
                "workspace root must be a directory".into(),
            ));
        }

        Ok(Self {
            workspace_root,
            max_file_size,
            max_line_count,
        })
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
                    "maximum": self.max_line_count,
                    "description": "Maximum number of lines to return. If omitted, reads to the end of the file."
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

                if value == 0 || value > self.max_line_count {
                    return Err(ToolError::InvalidArguments(format!(
                        "'line_count' must be between 1 and {}",
                        self.max_line_count
                    )));
                }

                value
            }
            None => usize::MAX,
        };

        let (resolved_path, content) =
            read_workspace_text_file(&self.workspace_root, path, self.max_file_size).await?;

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
            .expect("resolved_path must be inside workspace_root");

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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_tmp_dir() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = PathBuf::from("/tmp").join(format!("work-agent-text-read-{unique}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn reads_requested_line_range() {
        let root = make_tmp_dir();
        fs::write(root.join("notes.txt"), "alpha\nbeta\ngamma\ndelta\n").unwrap();

        let tool = TextFileReadTool::new(root, 1_048_576, 400).unwrap();
        let result = tool
            .execute(json!({
                "path": "notes.txt",
                "start_line": 2,
                "line_count": 2
            }))
            .await
            .unwrap();

        assert!(!result.is_error);
        assert_eq!(result.output["path"], json!("notes.txt"));
        assert_eq!(result.output["start_line"], json!(2));
        assert_eq!(result.output["end_line"], json!(3));
        assert_eq!(result.output["total_lines"], json!(4));
        assert_eq!(result.output["returned_lines"], json!(2));
        assert_eq!(result.output["truncated"], json!(true));
        assert_eq!(result.output["content"], json!("2 | beta\n3 | gamma"));
    }
}
