use crate::domain::error::tool_error::ToolError;
use crate::domain::port::tool::Tool;
use crate::domain::port::tool::ToolOutput;
use crate::infrastructure::util::path::{contains_parent_dir, normalize_path};
use async_trait::async_trait;
use glob::{MatchOptions, glob_with};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub struct FileSearchTool {
    workspace_root: PathBuf,
    max_results: usize,
}

impl FileSearchTool {
    pub fn new(workspace_root: impl Into<PathBuf>, max_results: usize) -> Result<Self, ToolError> {
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
            max_results,
        })
    }
}

#[async_trait]
impl Tool for FileSearchTool {
    fn name(&self) -> &str {
        "file_search"
    }

    fn description(&self) -> &str {
        "Search files by glob-style path pattern."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern to match files. Relative patterns are resolved from the workspace root. Example: src/**/*.rs"
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, arguments: Value) -> Result<ToolOutput, ToolError> {
        let pattern = arguments
            .get("pattern")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ToolError::InvalidArguments("missing or invalid 'pattern'".into()))?;

        // Security: reject absolute patterns and path traversal
        let pattern_path = Path::new(pattern);
        if pattern_path.is_absolute() {
            return Err(ToolError::PermissionDenied(
                "absolute patterns are not allowed".into(),
            ));
        }
        if contains_parent_dir(pattern_path) {
            return Err(ToolError::PermissionDenied(
                "path traversal is not allowed in patterns".into(),
            ));
        }

        let match_options = MatchOptions {
            case_sensitive: true,
            require_literal_separator: true,
            require_literal_leading_dot: false,
        };

        let search_pattern = self
            .workspace_root
            .join(pattern)
            .to_string_lossy()
            .to_string();

        let entries = glob_with(&search_pattern, match_options)
            .map_err(|err| ToolError::InvalidArguments(format!("invalid 'pattern': {err}")))?;

        let mut matches = Vec::new();

        for entry in entries {
            let Ok(path) = entry else {
                continue;
            };

            // Resolve symlinks and verify the result stays inside the workspace
            let Ok(canonical) = std::fs::canonicalize(&path) else {
                continue;
            };
            if !canonical.starts_with(&self.workspace_root) {
                continue;
            }
            if !canonical.is_file() {
                continue;
            }

            let relative = canonical
                .strip_prefix(&self.workspace_root)
                .map(normalize_path)
                .expect("canonical path must be inside workspace_root");
            matches.push(relative);
        }

        matches.sort();
        matches.dedup();
        let total_matches = matches.len();
        let truncated = total_matches > self.max_results;
        matches.truncate(self.max_results);

        Ok(ToolOutput::success(json!({
            "pattern": pattern,
            "total_matches": total_matches,
            "matches": matches,
            "truncated": truncated,
        })))
    }
}

#[cfg(test)]
mod tests {
    use crate::domain::model::tool_call::ToolCallOutputStatus;

    use super::*;
    use serde_json::json;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_tmp_dir() -> PathBuf {
        // Dummy directory
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("commander-file-search-{unique}"));
        fs::create_dir_all(&dir).unwrap();

        // Dummy files
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("src/lib.rs"), "").unwrap();
        fs::write(dir.join("src/main.rs"), "").unwrap();
        fs::write(dir.join("README.md"), "").unwrap();

        dir
    }

    #[tokio::test]
    async fn finds_matching_files() {
        let root = make_tmp_dir();
        let tool = FileSearchTool::new(root, 200).unwrap();

        let result = tool
            .execute(json!({
                "pattern": "src/**/*.rs"
            }))
            .await
            .unwrap();

        assert_eq!(result.status, ToolCallOutputStatus::Success);
        assert_eq!(
            result.output["matches"],
            json!(["src/lib.rs", "src/main.rs"])
        );

        // Cleanup
        fs::remove_dir_all(tool.workspace_root).unwrap();
    }
}
