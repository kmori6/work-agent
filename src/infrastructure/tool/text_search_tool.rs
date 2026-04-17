use crate::domain::error::tool_error::ToolError;
use crate::domain::model::tool::ToolExecutionResult;
use crate::domain::port::tool::Tool;
use crate::infrastructure::util::path::{contains_parent_dir, normalize_path};
use async_trait::async_trait;
use glob::{MatchOptions, Pattern};
use ignore::WalkBuilder;
use regex::RegexBuilder;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub struct TextSearchTool {
    workspace_root: PathBuf,
    max_file_size: u64,
    max_results: usize,
    max_context_lines: usize,
}

impl TextSearchTool {
    pub fn new(
        workspace_root: impl Into<PathBuf>,
        max_file_size: u64,
        max_results: usize,
        max_context_lines: usize,
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
            max_results,
            max_context_lines,
        })
    }
}

#[async_trait]
impl Tool for TextSearchTool {
    fn name(&self) -> &str {
        "text_search"
    }

    fn description(&self) -> &str {
        "Search text inside workspace files by regex and return matching lines with context."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Regex pattern to search for. Plain text works as-is."
                },
                "include": {
                    "type": "string",
                    "description": "Glob filter for files to search. Example: **/*.rs"
                },
                "context_lines": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": self.max_context_lines,
                    "description": "Number of surrounding lines around each match. Default is 0."
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, arguments: Value) -> Result<ToolExecutionResult, ToolError> {
        let query = arguments
            .get("query")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ToolError::InvalidArguments("missing or invalid 'query'".into()))?;

        let include = arguments
            .get("include")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());

        // gardrail for security: only allow relative glob patterns without path traversal
        if let Some(include) = include {
            let path = Path::new(include);
            if path.is_absolute() {
                return Err(ToolError::InvalidArguments(
                    "'include' must be a relative glob pattern".into(),
                ));
            }
            if contains_parent_dir(path) {
                return Err(ToolError::PermissionDenied(
                    "path traversal is not allowed in 'include'".into(),
                ));
            }
        }

        let include_matcher = match include {
            Some(pattern) => Some(Pattern::new(pattern).map_err(|err| {
                ToolError::InvalidArguments(format!("invalid 'include' pattern: {err}"))
            })?),
            None => None,
        };

        let context_lines = match arguments.get("context_lines") {
            Some(value) => {
                let value = value.as_u64().ok_or_else(|| {
                    ToolError::InvalidArguments("'context_lines' must be an integer".into())
                })?;
                let value = usize::try_from(value).map_err(|_| {
                    ToolError::InvalidArguments("'context_lines' is out of supported range".into())
                })?;
                if value > self.max_context_lines {
                    return Err(ToolError::InvalidArguments(format!(
                        "'context_lines' must be between 0 and {}",
                        self.max_context_lines
                    )));
                }
                value
            }
            None => 0,
        };

        let regex = RegexBuilder::new(query)
            .case_insensitive(true)
            .build()
            .map_err(|err| {
                ToolError::InvalidArguments(format!("invalid regex in 'query': {err}"))
            })?;

        let mut walker = WalkBuilder::new(&self.workspace_root);
        walker.hidden(true);
        walker.git_ignore(true);
        walker.git_exclude(true);
        walker.ignore(true);
        walker.parents(true);
        walker.follow_links(false);

        let glob_options = MatchOptions {
            case_sensitive: true,
            require_literal_separator: false,
            require_literal_leading_dot: true,
        };

        let mut total_matches = 0usize;
        let mut matches = Vec::new();

        for entry in walker.build() {
            let Ok(entry) = entry else { continue };

            let Some(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_file() {
                continue;
            }

            let logical_path = entry.path();

            let Ok(resolved) = std::fs::canonicalize(logical_path) else {
                continue;
            };
            if !resolved.starts_with(&self.workspace_root) {
                continue;
            }

            let Ok(relative) = logical_path.strip_prefix(&self.workspace_root) else {
                continue;
            };
            let relative = normalize_path(relative);

            if let Some(pattern) = &include_matcher
                && !pattern.matches_with(&relative, glob_options)
            {
                continue;
            }

            let Ok(metadata) = tokio::fs::metadata(&resolved).await else {
                continue;
            };
            if metadata.len() > self.max_file_size {
                continue;
            }

            let Ok(bytes) = tokio::fs::read(&resolved).await else {
                continue;
            };
            if bytes.contains(&0) {
                continue;
            }
            let Ok(content) = String::from_utf8(bytes) else {
                continue;
            };

            let lines: Vec<&str> = content.lines().collect();

            for (line_index, line) in lines.iter().enumerate() {
                if !regex.is_match(line) {
                    continue;
                }

                total_matches += 1;

                if matches.len() < self.max_results {
                    matches.push(json!({
                        "path": relative,
                        "line_number": line_index + 1,
                        "snippet": build_snippet(&lines, line_index, context_lines)
                    }));
                }
            }
        }

        Ok(ToolExecutionResult::success(json!({
            "total_matches": total_matches,
            "matches": matches
        })))
    }
}

fn build_snippet(lines: &[&str], line_index: usize, context_lines: usize) -> String {
    let start = line_index.saturating_sub(context_lines);
    let end = line_index
        .saturating_add(context_lines)
        .saturating_add(1)
        .min(lines.len());

    (start..end)
        .map(|index| format!("{} | {}", index + 1, lines[index]))
        .collect::<Vec<_>>()
        .join("\n")
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
        let dir = PathBuf::from("/tmp").join(format!("work-agent-text-search-{unique}"));
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(
            dir.join("src/main.rs"),
            "fn main() {\n    println!(\"hello\");\n}\n",
        )
        .unwrap();
        fs::write(dir.join("README.md"), "# Title\nSome description\n").unwrap();
        dir
    }

    #[tokio::test]
    async fn finds_matching_lines() {
        let root = make_tmp_dir();
        let tool = TextSearchTool::new(root, 1_048_576, 200, 10).unwrap();

        let result = tool.execute(json!({ "query": "fn main" })).await.unwrap();

        assert!(!result.is_error);
        assert_eq!(result.output["total_matches"], json!(1));
        assert_eq!(result.output["matches"][0]["path"], json!("src/main.rs"));
        assert_eq!(result.output["matches"][0]["line_number"], json!(1));
    }
}
