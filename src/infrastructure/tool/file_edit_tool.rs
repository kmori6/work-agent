use crate::domain::error::tool_error::ToolError;
use crate::domain::model::tool::ToolExecutionResult;
use crate::domain::port::tool::{Tool, ToolExecutionPolicy};
use crate::infrastructure::util::path::{normalize_path, read_workspace_text_file};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::path::PathBuf;

pub struct FileEditTool {
    workspace_root: PathBuf,
    max_file_size: u64,
}

impl FileEditTool {
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
impl Tool for FileEditTool {
    fn name(&self) -> &str {
        "file_edit"
    }

    fn description(&self) -> &str {
        "Replace exactly one matched text block in a UTF-8 file in the workspace. Use file_write for full rewrites."
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
                    "description": "Exact text to find and replace. It must match exactly once in the file."
                },
                "new_text": {
                    "type": "string",
                    "description": "Replacement text. Use an empty string to delete the matched text."
                }
            },
            "required": ["path", "old_text", "new_text"]
        })
    }

    fn execution_policy(&self, _arguments: &Value) -> ToolExecutionPolicy {
        ToolExecutionPolicy::Ask
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

        let (resolved_path, content) =
            read_workspace_text_file(&self.workspace_root, path, self.max_file_size).await?;

        let match_count = content.matches(old_text).count();

        if match_count == 0 {
            return Err(ToolError::ExecutionFailed(
                "'old_text' was not found in the file".into(),
            ));
        }

        if match_count > 1 {
            return Err(ToolError::ExecutionFailed(format!(
                "'old_text' matched {match_count} times; provide more surrounding context so it matches exactly once"
            )));
        }

        let updated_content = content.replacen(old_text, new_text, 1);

        tokio::fs::write(&resolved_path, &updated_content)
            .await
            .map_err(|err| ToolError::ExecutionFailed(format!("failed to write file: {err}")))?;

        let relative_path = resolved_path
            .strip_prefix(&self.workspace_root)
            .map(normalize_path)
            .expect("resolved_path must be inside workspace_root");

        Ok(ToolExecutionResult::success(json!({
            "path": relative_path
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
        let dir = PathBuf::from("/tmp").join(format!("commander-file-edit-{unique}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn replaces_exact_match() {
        let root = make_tmp_dir();
        fs::write(root.join("hello.txt"), "hello world").unwrap();

        let tool = FileEditTool::new(root.clone(), 1_048_576).unwrap();
        let result = tool
            .execute(json!({
                "path": "hello.txt",
                "old_text": "hello",
                "new_text": "goodbye"
            }))
            .await
            .unwrap();

        assert!(!result.is_error);
        assert_eq!(result.output["path"], json!("hello.txt"));
        assert_eq!(
            fs::read_to_string(root.join("hello.txt")).unwrap(),
            "goodbye world"
        );
    }

    #[tokio::test]
    async fn deletes_text_when_new_text_is_empty() {
        let root = make_tmp_dir();
        fs::write(root.join("hello.txt"), "hello world").unwrap();

        let tool = FileEditTool::new(root.clone(), 1_048_576).unwrap();
        let result = tool
            .execute(json!({
                "path": "hello.txt",
                "old_text": " world",
                "new_text": ""
            }))
            .await
            .unwrap();

        assert!(!result.is_error);
        assert_eq!(result.output["path"], json!("hello.txt"));
        assert_eq!(fs::read_to_string(root.join("hello.txt")).unwrap(), "hello");
    }

    #[tokio::test]
    async fn fails_when_old_text_is_missing() {
        let root = make_tmp_dir();
        fs::write(root.join("hello.txt"), "hello world").unwrap();

        let tool = FileEditTool::new(root.clone(), 1_048_576).unwrap();
        let err = tool
            .execute(json!({
                "path": "hello.txt",
                "old_text": "goodbye",
                "new_text": "hello"
            }))
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            ToolError::ExecutionFailed(message) if message.contains("not found")
        ));
        assert_eq!(
            fs::read_to_string(root.join("hello.txt")).unwrap(),
            "hello world"
        );
    }

    #[tokio::test]
    async fn fails_when_old_text_matches_multiple_times() {
        let root = make_tmp_dir();
        fs::write(root.join("hello.txt"), "hello world\nhello team").unwrap();

        let tool = FileEditTool::new(root.clone(), 1_048_576).unwrap();
        let err = tool
            .execute(json!({
                "path": "hello.txt",
                "old_text": "hello",
                "new_text": "goodbye"
            }))
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            ToolError::ExecutionFailed(message) if message.contains("matched 2 times")
        ));
        assert_eq!(
            fs::read_to_string(root.join("hello.txt")).unwrap(),
            "hello world\nhello team"
        );
    }

    #[tokio::test]
    async fn fails_when_old_text_is_empty() {
        let root = make_tmp_dir();
        fs::write(root.join("hello.txt"), "hello world").unwrap();

        let tool = FileEditTool::new(root, 1_048_576).unwrap();
        let err = tool
            .execute(json!({
                "path": "hello.txt",
                "old_text": "",
                "new_text": "goodbye"
            }))
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            ToolError::InvalidArguments(message) if message.contains("must not be empty")
        ));
    }

    #[tokio::test]
    async fn fails_when_old_text_and_new_text_are_identical() {
        let root = make_tmp_dir();
        fs::write(root.join("hello.txt"), "hello world").unwrap();

        let tool = FileEditTool::new(root, 1_048_576).unwrap();
        let err = tool
            .execute(json!({
                "path": "hello.txt",
                "old_text": "hello",
                "new_text": "hello"
            }))
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            ToolError::InvalidArguments(message) if message.contains("must be different")
        ));
    }
}
