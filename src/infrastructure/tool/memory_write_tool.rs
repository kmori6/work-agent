use crate::domain::error::tool_error::ToolError;
use crate::domain::port::tool::Tool;
use crate::domain::port::tool::ToolOutput;
use crate::domain::service::memory_index_service::MemoryIndexService;
use crate::infrastructure::util::path::normalize_path;
use async_trait::async_trait;
use chrono::{Local, NaiveDate};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;

pub struct MemoryWriteTool {
    workspace_root: PathBuf,
    memory_index_service: Arc<MemoryIndexService>,
}

impl MemoryWriteTool {
    pub fn new(
        workspace_root: impl Into<PathBuf>,
        memory_index_service: Arc<MemoryIndexService>,
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
            memory_index_service,
        })
    }

    fn memory_root(&self) -> PathBuf {
        self.workspace_root.join(".commander").join("memory")
    }

    fn resolve_target(
        &self,
        target: &str,
        journal_date: Option<&str>,
    ) -> Result<PathBuf, ToolError> {
        match target {
            "memory" => Ok(self.memory_root().join("MEMORY.md")),
            "journal" => {
                let date = match journal_date {
                    Some(value) if !value.trim().is_empty() => {
                        NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d").map_err(|_| {
                            ToolError::InvalidArguments(
                                "'journal_date' must use YYYY-MM-DD format".into(),
                            )
                        })?;
                        value.trim().to_string()
                    }
                    _ => Local::now().format("%Y-%m-%d").to_string(),
                };

                Ok(self
                    .memory_root()
                    .join("journals")
                    .join(format!("{date}.md")))
            }
            _ => Err(ToolError::InvalidArguments(
                "'target' must be either 'memory' or 'journal'".into(),
            )),
        }
    }
}

#[async_trait]
impl Tool for MemoryWriteTool {
    fn name(&self) -> &str {
        "memory_write"
    }

    fn description(&self) -> &str {
        "Append Markdown to Commander memory or the daily journal."
    }

    fn parameters(&self) -> Value {
        json!({
          "type": "object",
          "properties": {
            "target": {
              "type": "string",
              "enum": ["memory", "journal"],
              "description": "Where to write the memory. Use memory for durable long-term facts and journal for work notes, decisions, and daily context."
            },
            "content": {
              "type": "string",
              "description": "The memory content to append as Markdown. Write the final memory text directly, not an instruction to remember it."
            },
            "journal_date": {
              "type": "string",
              "description": "Optional date for journal entries in YYYY-MM-DD format. Only used when target is journal. Defaults to today's local date."
            }
          },
          "required": ["target", "content"]
        })
    }

    async fn execute(&self, arguments: Value) -> Result<ToolOutput, ToolError> {
        let target = arguments
            .get("target")
            .and_then(|value| value.as_str())
            .ok_or_else(|| ToolError::InvalidArguments("missing or invalid 'target'".into()))?;

        let content = arguments
            .get("content")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ToolError::InvalidArguments("missing or invalid 'content'".into()))?;

        let journal_date = arguments
            .get("journal_date")
            .and_then(|value| value.as_str());
        let path = self.resolve_target(target, journal_date)?;

        let parent = path.parent().ok_or_else(|| {
            ToolError::ExecutionFailed("memory path must include a parent directory".into())
        })?;

        tokio::fs::create_dir_all(parent).await.map_err(|err| {
            ToolError::ExecutionFailed(format!("failed to create memory directory: {err}"))
        })?;

        let existing = match tokio::fs::read_to_string(&path).await {
            Ok(content) => content,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(err) => {
                return Err(ToolError::ExecutionFailed(format!(
                    "failed to read memory file: {err}"
                )));
            }
        };

        let separator = if existing.trim().is_empty() {
            ""
        } else if existing.ends_with("\n\n") {
            ""
        } else if existing.ends_with('\n') {
            "\n"
        } else {
            "\n\n"
        };

        let entry = format!("{separator}{}\n", content.trim());

        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
            .map_err(|err| {
                ToolError::ExecutionFailed(format!("failed to open memory file: {err}"))
            })?;

        file.write_all(entry.as_bytes()).await.map_err(|err| {
            ToolError::ExecutionFailed(format!("failed to write memory file: {err}"))
        })?;

        let relative_path = path
            .strip_prefix(&self.workspace_root)
            .map(normalize_path)
            .expect("memory path must be inside workspace root");

        let full_content = format!("{existing}{entry}");

        let (index_status, index_error) = if target == "journal" {
            match self
                .memory_index_service
                .rebuild_path_index(&relative_path, &full_content)
                .await
            {
                Ok(_) => ("indexed", None),
                Err(err) => ("stale", Some(err.to_string())),
            }
        } else {
            ("skipped", None)
        };

        let mut output = json!({
            "path": relative_path,
            "index_status": index_status
        });

        if let Some(error) = index_error {
            output["index_error"] = json!(error);
        }

        Ok(ToolOutput::success(output))
    }
}
