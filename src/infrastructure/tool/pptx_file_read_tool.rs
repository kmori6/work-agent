use crate::domain::error::tool_error::ToolError;
use crate::domain::model::tool::ToolExecutionResult;
use crate::domain::port::tool::Tool;
use crate::infrastructure::util::path::{normalize_path, resolve_workspace_file_path};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::path::PathBuf;
use tokio::process::Command;

pub struct PptxReadTool {
    workspace_root: PathBuf,
}

impl PptxReadTool {
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
impl Tool for PptxReadTool {
    fn name(&self) -> &str {
        "pptx_file_read"
    }

    fn description(&self) -> &str {
        "Read a PowerPoint (.pptx) file from the workspace and convert it to Markdown text using markitdown."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the .pptx file. Relative paths are resolved from the workspace root."
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

        let resolved_path = resolve_workspace_file_path(&self.workspace_root, path)?;

        let output = Command::new("markitdown")
            .arg(&resolved_path)
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

        let relative_path = resolved_path
            .strip_prefix(&self.workspace_root)
            .map(normalize_path)
            .expect("resolved_path must be inside workspace_root");

        Ok(ToolExecutionResult::success(json!({
            "path": relative_path,
            "content": content
        })))
    }
}
