use crate::domain::error::tool_error::ToolError;
use crate::domain::model::tool::ToolExecutionResult;
use crate::domain::port::tool::Tool;
use crate::infrastructure::util::path::{
    has_parent_traversal, normalize_path, resolve_workspace_directory_path,
};
use async_trait::async_trait;
use glob::{MatchOptions, Pattern};
use ignore::WalkBuilder;
use regex::{Regex, RegexBuilder};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const DEFAULT_MAX_RESULTS: usize = 20;
const MAX_RESULTS: usize = 200;
const MAX_CONTEXT_LINES: usize = 10;
const MAX_FILE_SIZE_BYTES: u64 = 1_048_576;

pub struct TextSearchTool {
    workspace_root: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MatchMode {
    Substring,
    Regex,
    Exact,
}

impl MatchMode {
    fn parse(value: Option<&Value>) -> Result<Self, ToolError> {
        match value.and_then(|v| v.as_str()) {
            None => Ok(Self::Substring),
            Some("substring") => Ok(Self::Substring),
            Some("regex") => Ok(Self::Regex),
            Some("exact") => Ok(Self::Exact),
            Some(other) => Err(ToolError::InvalidArguments(format!(
                "'match_mode' must be one of: substring, regex, exact. got: {other}"
            ))),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Substring => "substring",
            Self::Regex => "regex",
            Self::Exact => "exact",
        }
    }
}

impl TextSearchTool {
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

    fn line_matches(
        line: &str,
        query: &str,
        query_lower: &str,
        match_mode: MatchMode,
        case_sensitive: bool,
        regex: Option<&Regex>,
    ) -> bool {
        match match_mode {
            MatchMode::Substring => {
                if case_sensitive {
                    line.contains(query)
                } else {
                    line.to_lowercase().contains(query_lower)
                }
            }
            MatchMode::Regex => regex.is_some_and(|regex| regex.is_match(line)),
            MatchMode::Exact => {
                if case_sensitive {
                    line == query
                } else {
                    line.to_lowercase() == query_lower
                }
            }
        }
    }
}

#[async_trait]
impl Tool for TextSearchTool {
    fn name(&self) -> &str {
        "text_search"
    }

    fn description(&self) -> &str {
        "Search text inside workspace files and return matching lines with optional context."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Text or pattern to search for."
                },
                "match_mode": {
                    "type": "string",
                    "enum": ["substring", "regex", "exact"],
                    "description": "How to interpret query. 'exact' means whole-line match. Default is substring."
                },
                "base_path": {
                    "type": "string",
                    "description": "Base directory to search from. Relative paths are resolved from the workspace root. Absolute paths are allowed only if they stay inside the workspace. Default is current workspace root."
                },
                "file_pattern": {
                    "type": "string",
                    "description": "Optional glob filter for candidate files. Example: **/*.rs"
                },
                "case_sensitive": {
                    "type": "boolean",
                    "description": "Whether matching is case sensitive. Default is false."
                },
                "context_lines": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": MAX_CONTEXT_LINES,
                    "description": "Number of surrounding lines to include around each match. Default is 0."
                },
                "max_results": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_RESULTS,
                    "description": "Maximum number of matches to return. Default is 50."
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

        let match_mode = MatchMode::parse(arguments.get("match_mode"))?;

        let base_path = arguments
            .get("base_path")
            .and_then(|v| v.as_str())
            .unwrap_or(".");

        let file_pattern = arguments
            .get("file_pattern")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());

        if let Some(file_pattern) = file_pattern {
            let path = Path::new(file_pattern);
            if path.is_absolute() {
                return Err(ToolError::InvalidArguments(
                    "'file_pattern' must be relative to 'base_path'".into(),
                ));
            }

            if has_parent_traversal(path) {
                return Err(ToolError::PermissionDenied(
                    "path traversal is not allowed in 'file_pattern'".into(),
                ));
            }
        }

        let file_pattern_matcher = match file_pattern {
            Some(pattern) => Some(Pattern::new(pattern).map_err(|err| {
                ToolError::InvalidArguments(format!("invalid 'file_pattern': {err}"))
            })?),
            None => None,
        };

        let case_sensitive = arguments
            .get("case_sensitive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let context_lines = match arguments.get("context_lines") {
            Some(value) => {
                let value = value.as_u64().ok_or_else(|| {
                    ToolError::InvalidArguments("'context_lines' must be an integer".into())
                })?;

                let value = usize::try_from(value).map_err(|_| {
                    ToolError::InvalidArguments("'context_lines' is out of supported range".into())
                })?;

                if value > MAX_CONTEXT_LINES {
                    return Err(ToolError::InvalidArguments(format!(
                        "'context_lines' must be between 0 and {MAX_CONTEXT_LINES}"
                    )));
                }

                value
            }
            None => 0,
        };

        let max_results = match arguments.get("max_results") {
            Some(value) => {
                let value = value.as_u64().ok_or_else(|| {
                    ToolError::InvalidArguments("'max_results' must be an integer".into())
                })?;

                let value = usize::try_from(value).map_err(|_| {
                    ToolError::InvalidArguments("'max_results' is out of supported range".into())
                })?;

                if value == 0 || value > MAX_RESULTS {
                    return Err(ToolError::InvalidArguments(format!(
                        "'max_results' must be between 1 and {MAX_RESULTS}"
                    )));
                }

                value
            }
            None => DEFAULT_MAX_RESULTS,
        };

        let search_root = resolve_workspace_directory_path(&self.workspace_root, base_path)?;

        let regex = if match_mode == MatchMode::Regex {
            Some(
                RegexBuilder::new(query)
                    .case_insensitive(!case_sensitive)
                    .build()
                    .map_err(|err| {
                        ToolError::InvalidArguments(format!("invalid regex query: {err}"))
                    })?,
            )
        } else {
            None
        };

        let query_lower = if case_sensitive {
            String::new()
        } else {
            query.to_lowercase()
        };

        let mut walker = WalkBuilder::new(&search_root);
        walker.hidden(true);
        walker.git_ignore(true);
        walker.git_exclude(true);
        walker.ignore(true);
        walker.parents(true);
        walker.follow_links(false);

        let file_match_options = MatchOptions {
            case_sensitive: true,
            require_literal_separator: false,
            require_literal_leading_dot: true,
        };

        let mut searched_files_count = 0usize;
        let mut skipped_files_count = 0usize;
        let mut total_matches = 0usize;
        let mut matched_files = BTreeSet::new();
        let mut matches = Vec::new();

        for entry in walker.build() {
            let Ok(entry) = entry else {
                continue;
            };

            let logical_path = entry.path();

            let Some(file_type) = entry.file_type() else {
                continue;
            };

            if !file_type.is_file() {
                continue;
            }

            let Ok(resolved_path) = std::fs::canonicalize(logical_path) else {
                skipped_files_count += 1;
                continue;
            };

            if !resolved_path.starts_with(&self.workspace_root) {
                skipped_files_count += 1;
                continue;
            }

            let Ok(relative_to_search_root) = logical_path.strip_prefix(&search_root) else {
                skipped_files_count += 1;
                continue;
            };

            let Ok(relative_to_workspace) = logical_path.strip_prefix(&self.workspace_root) else {
                skipped_files_count += 1;
                continue;
            };

            let relative_to_search_root = normalize_path(relative_to_search_root);
            let relative_to_workspace = normalize_path(relative_to_workspace);

            if let Some(pattern) = &file_pattern_matcher
                && !pattern.matches_with(&relative_to_search_root, file_match_options)
            {
                continue;
            }

            searched_files_count += 1;

            let Ok(metadata) = tokio::fs::metadata(&resolved_path).await else {
                skipped_files_count += 1;
                continue;
            };

            if metadata.len() > MAX_FILE_SIZE_BYTES {
                skipped_files_count += 1;
                continue;
            }

            let Ok(bytes) = tokio::fs::read(&resolved_path).await else {
                skipped_files_count += 1;
                continue;
            };

            if bytes.contains(&0) {
                skipped_files_count += 1;
                continue;
            }

            let Ok(content) = String::from_utf8(bytes) else {
                skipped_files_count += 1;
                continue;
            };

            let lines: Vec<&str> = content.lines().collect();

            for (line_index, line) in lines.iter().enumerate() {
                if !Self::line_matches(
                    line,
                    query,
                    &query_lower,
                    match_mode,
                    case_sensitive,
                    regex.as_ref(),
                ) {
                    continue;
                }

                total_matches += 1;
                matched_files.insert(relative_to_workspace.clone());

                if matches.len() < max_results {
                    matches.push(json!({
                        "path": relative_to_workspace,
                        "line_number": line_index + 1,
                        "line": line,
                        "snippet": Self::build_snippet(&lines, line_index, context_lines)
                    }));
                }
            }
        }

        let base_path = search_root
            .strip_prefix(&self.workspace_root)
            .map(normalize_path)
            .unwrap_or_else(|_| ".".to_string());

        Ok(ToolExecutionResult::success(json!({
            "query": query,
            "match_mode": match_mode.as_str(),
            "base_path": base_path,
            "file_pattern": file_pattern,
            "case_sensitive": case_sensitive,
            "context_lines": context_lines,
            "searched_files_count": searched_files_count,
            "skipped_files_count": skipped_files_count,
            "matched_files_count": matched_files.len(),
            "total_matches": total_matches,
            "returned_matches": matches.len(),
            "truncated": total_matches > matches.len(),
            "matched_files": matched_files.into_iter().collect::<Vec<_>>(),
            "matches": matches
        })))
    }
}
