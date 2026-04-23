use crate::domain::error::tool_error::ToolError;
use crate::domain::model::tool::ToolExecutionResult;
use crate::domain::port::tool::{Tool, ToolExecutionPolicy};
use crate::infrastructure::util::path::{contains_parent_dir, normalize_path};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub struct FileWriteTool {
    workspace_root: PathBuf,
}

impl FileWriteTool {
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

    fn resolve_target_path(&self, path: &str) -> Result<PathBuf, ToolError> {
        let path = path.trim();
        if path.is_empty() {
            return Err(ToolError::InvalidArguments(
                "'path' must not be empty".into(),
            ));
        }

        let path = Path::new(path);
        if !path.is_absolute() && contains_parent_dir(path) {
            return Err(ToolError::PermissionDenied(
                "path traversal is not allowed in relative paths".into(),
            ));
        }

        let candidate = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.workspace_root.join(path)
        };

        if !candidate.starts_with(&self.workspace_root) {
            return Err(ToolError::PermissionDenied(
                "'path' must stay inside the workspace".into(),
            ));
        }

        Ok(candidate)
    }
}

#[async_trait]
impl Tool for FileWriteTool {
    fn name(&self) -> &str {
        "file_write"
    }

    fn description(&self) -> &str {
        "Write full UTF-8 text content to a file in the workspace. Creates parent directories automatically and replaces the file if it already exists."
    }

    fn parameters(&self) -> Value {
        json!({
          "type": "object",
          "properties": {
            "path": {
              "type": "string",
              "description": "Path to the UTF-8 text file to write. Relative paths are resolved from the workspace root. Absolute paths are allowed only if they stay inside the workspace."
            },
            "content": {
              "type": "string",
              "description": "Full UTF-8 text content to write."
            }
          },
          "required": ["path", "content"]
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

        let content = arguments
            .get("content")
            .and_then(|value| value.as_str())
            .ok_or_else(|| ToolError::InvalidArguments("missing or invalid 'content'".into()))?;

        let content_bytes = content.len() as u64;

        let target_path = self.resolve_target_path(path)?;

        let parent = target_path
            .parent()
            .ok_or_else(|| ToolError::InvalidArguments("'path' must include a file name".into()))?;

        tokio::fs::create_dir_all(parent).await.map_err(|err| {
            ToolError::ExecutionFailed(format!("failed to create directory: {err}"))
        })?;

        let resolved_parent = tokio::fs::canonicalize(parent).await.map_err(|err| {
            ToolError::ExecutionFailed(format!("failed to resolve parent directory: {err}"))
        })?;

        if !resolved_parent.starts_with(&self.workspace_root) {
            return Err(ToolError::PermissionDenied(
                "'path' resolved outside the workspace".into(),
            ));
        }

        let file_name = target_path
            .file_name()
            .ok_or_else(|| ToolError::InvalidArguments("'path' must include a file name".into()))?;
        let resolved_target = resolved_parent.join(file_name);

        tokio::fs::write(&resolved_target, content)
            .await
            .map_err(|err| ToolError::ExecutionFailed(format!("failed to write file: {err}")))?;

        let relative_path = resolved_target
            .strip_prefix(&self.workspace_root)
            .map(normalize_path)
            .expect("resolved_target must be inside workspace_root");

        Ok(ToolExecutionResult::success(json!({
            "path": relative_path,
            "bytes_written": content_bytes
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
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/test-tmp")
            .join(format!("commander-file-write-{unique}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn creates_new_file_with_parent_directories() {
        let root = make_tmp_dir();
        let tool = FileWriteTool::new(root.clone()).unwrap();

        let result = tool
            .execute(json!({
                "path": "data/tasks/meeting.md",
                "content": "# Action Items\n\n- [ ] Follow up"
            }))
            .await
            .unwrap();

        assert!(!result.is_error);
        assert_eq!(result.output["path"], json!("data/tasks/meeting.md"));
        assert_eq!(
            fs::read_to_string(root.join("data/tasks/meeting.md")).unwrap(),
            "# Action Items\n\n- [ ] Follow up"
        );
    }

    #[tokio::test]
    async fn replaces_existing_file() {
        let root = make_tmp_dir();
        fs::create_dir_all(root.join("data")).unwrap();
        fs::write(root.join("data/existing.md"), "old").unwrap();

        let tool = FileWriteTool::new(root.clone()).unwrap();
        let result = tool
            .execute(json!({
                "path": "data/existing.md",
                "content": "new"
            }))
            .await
            .unwrap();

        assert!(!result.is_error);
        assert_eq!(result.output["path"], json!("data/existing.md"));
        assert_eq!(result.output["bytes_written"], json!(3));
        assert_eq!(
            fs::read_to_string(root.join("data/existing.md")).unwrap(),
            "new"
        );
    }
}
