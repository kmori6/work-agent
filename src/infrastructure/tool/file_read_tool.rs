use crate::domain::error::tool_error::ToolError;
use crate::domain::port::tool::Tool;
use crate::domain::port::tool::ToolOutput;
use crate::infrastructure::util::path::{normalize_path, resolve_workspace_file_path};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::io::Read;
use std::path::PathBuf;
use tokio::process::Command;

pub struct FileReadTool {
    workspace_root: PathBuf,
    max_file_size: u64,
}

impl FileReadTool {
    pub fn new(workspace_root: impl Into<PathBuf>, max_file_size: u64) -> Result<Self, ToolError> {
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
        })
    }
}

#[async_trait]
impl Tool for FileReadTool {
    fn name(&self) -> &str {
        "file_read"
    }

    fn description(&self) -> &str {
        "Read a file from the workspace. Text files are read directly; binary files (PDF, DOCX, PPTX, XLSX, etc.) are automatically converted to Markdown via markitdown."
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
                    "description": "Maximum number of lines to return. If omitted, reads to the end of the file."
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, arguments: Value) -> Result<ToolOutput, ToolError> {
        let path = parse_path_argument(&arguments)?;
        let start_line = parse_start_line_argument(&arguments)?;
        let line_count = parse_line_count_argument(&arguments)?;

        let resolved_path = resolve_workspace_file_path(&self.workspace_root, &path)?;
        let content = if is_text_file(&resolved_path)? {
            read_content_from_text_file(&resolved_path, self.max_file_size).await?
        } else {
            read_content_from_binary_file(&resolved_path).await?
        };

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

        Ok(ToolOutput::success(json!({
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

fn parse_path_argument(value: &Value) -> Result<String, ToolError> {
    value
        .get("path")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| ToolError::InvalidArguments("missing or invalid 'path'".into()))
}

fn parse_start_line_argument(value: &Value) -> Result<usize, ToolError> {
    match value.get("start_line") {
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

            Ok(value)
        }
        None => Ok(1),
    }
}

fn parse_line_count_argument(value: &Value) -> Result<usize, ToolError> {
    match value.get("line_count") {
        Some(value) => {
            let value = value.as_u64().ok_or_else(|| {
                ToolError::InvalidArguments("'line_count' must be an integer".into())
            })?;
            let value = usize::try_from(value).map_err(|_| {
                ToolError::InvalidArguments("'line_count' is out of supported range".into())
            })?;

            if value == 0 {
                return Err(ToolError::InvalidArguments(
                    "'line_count' must be greater than or equal to 1".into(),
                ));
            }

            Ok(value)
        }
        None => Ok(usize::MAX),
    }
}

fn is_text_file(path: &PathBuf) -> Result<bool, ToolError> {
    // Read a head portion of the file (up to 8KB)
    let mut buf = [0u8; 8192];
    let mut f = std::fs::File::open(path).map_err(|err| {
        ToolError::Unavailable(format!("failed to open file for type checking: {err}"))
    })?;
    let n = f.read(&mut buf).map_err(|err| {
        ToolError::Unavailable(format!("failed to read file for type checking: {err}"))
    })?;
    // Null bytes (0x00) -> likely binary
    Ok(!buf[..n].contains(&0))
}

async fn read_content_from_text_file(
    path: &PathBuf,
    max_file_size: u64,
) -> Result<String, ToolError> {
    let metadata = tokio::fs::metadata(&path).await.map_err(|err| {
        ToolError::ExecutionFailed(format!("failed to read file metadata: {err}"))
    })?;

    if metadata.len() > max_file_size {
        return Err(ToolError::ExecutionFailed(format!(
            "file is too large: {} bytes (max: {})",
            metadata.len(),
            max_file_size
        )));
    }

    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|err| ToolError::ExecutionFailed(format!("failed to read file: {err}")))?;

    if bytes.contains(&0) {
        return Err(ToolError::ExecutionFailed(
            "file appears to be binary".into(),
        ));
    }

    let content = String::from_utf8(bytes)
        .map_err(|_| ToolError::ExecutionFailed("file is not valid UTF-8 text".into()))?;

    Ok(content)
}

async fn read_content_from_binary_file(path: &PathBuf) -> Result<String, ToolError> {
    let output = Command::new("markitdown")
        .arg(path)
        .output()
        .await
        .map_err(|err| {
            ToolError::ExecutionFailed(format!("failed to execute markitdown: {err}"))
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ToolError::ExecutionFailed(format!(
            "markitdown exited with {}: {stderr}",
            output.status
        )));
    }

    let content = String::from_utf8_lossy(&output.stdout).into_owned();

    if content.trim().is_empty() {
        return Err(ToolError::ExecutionFailed(
            "markitdown returned empty output. The file may be a scanned document requiring OCR."
                .into(),
        ));
    }

    Ok(content)
}

#[cfg(test)]
mod tests {
    use crate::domain::model::tool_call::ToolCallOutputStatus;

    use super::*;
    use serde_json::json;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_tmp_dir() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = PathBuf::from("/tmp").join(format!("commander-text-read-{unique}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn reads_requested_line_range() {
        let root = make_tmp_dir();
        fs::write(root.join("notes.txt"), "alpha\nbeta\ngamma\ndelta\n").unwrap();

        let tool = FileReadTool::new(root, 1_048_576).unwrap();
        let result = tool
            .execute(json!({
                "path": "notes.txt",
                "start_line": 2,
                "line_count": 2
            }))
            .await
            .unwrap();

        assert_eq!(result.status, ToolCallOutputStatus::Success);
        assert_eq!(result.output["path"], json!("notes.txt"));
        assert_eq!(result.output["start_line"], json!(2));
        assert_eq!(result.output["end_line"], json!(3));
        assert_eq!(result.output["total_lines"], json!(4));
        assert_eq!(result.output["returned_lines"], json!(2));
        assert_eq!(result.output["truncated"], json!(true));
        assert_eq!(result.output["content"], json!("2 | beta\n3 | gamma"));
    }
}
