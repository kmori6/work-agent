use crate::domain::error::tool_error::ToolError;
use crate::domain::model::tool::ToolExecutionResult;
use crate::domain::port::tool::Tool;
use crate::infrastructure::util::path::{
    has_parent_traversal, normalize_path, resolve_workspace_directory_path,
};
use async_trait::async_trait;
use glob::{MatchOptions, Pattern};
use ignore::WalkBuilder;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub struct FileSearchTool {
    workspace_root: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchKind {
    File,
    Directory,
    Any,
}

impl SearchKind {
    fn parse(value: Option<&Value>) -> Result<Self, ToolError> {
        match value.and_then(|v| v.as_str()) {
            None => Ok(Self::File),
            Some("file") => Ok(Self::File),
            Some("directory") => Ok(Self::Directory),
            Some("any") => Ok(Self::Any),
            Some(other) => Err(ToolError::InvalidArguments(format!(
                "'kind' must be one of: file, directory, any. got: {other}"
            ))),
        }
    }

    fn matches(self, is_file: bool, is_dir: bool) -> bool {
        match self {
            Self::File => is_file,
            Self::Directory => is_dir,
            Self::Any => is_file || is_dir,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Directory => "directory",
            Self::Any => "any",
        }
    }
}

impl FileSearchTool {
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
impl Tool for FileSearchTool {
    fn name(&self) -> &str {
        "file_search"
    }

    fn description(&self) -> &str {
        "Search files or directories in the workspace by glob-style path pattern."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern relative to base_path. Example: src/**/*.rs"
                },
                "base_path": {
                    "type": "string",
                    "description": "Base directory to search from. Relative paths are resolved from the workspace root. Absolute paths are allowed only if they stay inside the workspace. Default is current workspace root."
                },
                "kind": {
                    "type": "string",
                    "enum": ["file", "directory", "any"],
                    "description": "Which kinds of paths to return. Default is file."
                },
                "max_results": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 200,
                    "description": "Maximum number of matches to return. Default is 50."
                },
                "include_hidden": {
                    "type": "boolean",
                    "description": "Whether to include hidden files and directories. Default is false."
                },
                "respect_gitignore": {
                    "type": "boolean",
                    "description": "Whether to respect .gitignore and .ignore rules. Default is true."
                },
                "max_depth": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Optional maximum directory depth."
                },
                "follow_symlinks": {
                    "type": "boolean",
                    "description": "Whether to follow symbolic links. Default is false."
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, arguments: Value) -> Result<ToolExecutionResult, ToolError> {
        // Keep parsing strict so the model gets fast feedback when it calls the tool incorrectly.
        let pattern = arguments
            .get("pattern")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ToolError::InvalidArguments("missing or invalid 'pattern'".into()))?;

        let pattern_path = Path::new(pattern);
        if pattern_path.is_absolute() {
            return Err(ToolError::InvalidArguments(
                "'pattern' must be relative to 'base_path'".into(),
            ));
        }

        if has_parent_traversal(pattern_path) {
            return Err(ToolError::PermissionDenied(
                "path traversal is not allowed in 'pattern'".into(),
            ));
        }

        let matcher = Pattern::new(pattern)
            .map_err(|err| ToolError::InvalidArguments(format!("invalid 'pattern': {err}")))?;

        let base_path = arguments
            .get("base_path")
            .and_then(|v| v.as_str())
            .unwrap_or(".");

        let kind = SearchKind::parse(arguments.get("kind"))?;

        let max_results = match arguments.get("max_results") {
            Some(value) => {
                let value = value.as_u64().ok_or_else(|| {
                    ToolError::InvalidArguments("'max_results' must be an integer".into())
                })?;

                let value = usize::try_from(value).map_err(|_| {
                    ToolError::InvalidArguments("'max_results' is out of supported range".into())
                })?;

                if value == 0 || value > 200 {
                    return Err(ToolError::InvalidArguments(
                        "'max_results' must be between 1 and 200".into(),
                    ));
                }

                value
            }
            None => 50,
        };

        let include_hidden = arguments
            .get("include_hidden")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let respect_gitignore = arguments
            .get("respect_gitignore")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let max_depth = match arguments.get("max_depth") {
            Some(value) => {
                let value = value.as_u64().ok_or_else(|| {
                    ToolError::InvalidArguments("'max_depth' must be an integer".into())
                })?;

                let value = usize::try_from(value).map_err(|_| {
                    ToolError::InvalidArguments("'max_depth' is out of supported range".into())
                })?;

                if value == 0 {
                    return Err(ToolError::InvalidArguments(
                        "'max_depth' must be greater than 0".into(),
                    ));
                }

                Some(value)
            }
            None => None,
        };

        let follow_symlinks = arguments
            .get("follow_symlinks")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let search_root = resolve_workspace_directory_path(&self.workspace_root, base_path)?;

        let mut walker = WalkBuilder::new(&search_root);
        walker.hidden(!include_hidden);
        walker.git_ignore(respect_gitignore);
        walker.git_exclude(respect_gitignore);
        walker.ignore(respect_gitignore);
        walker.parents(respect_gitignore);
        walker.follow_links(follow_symlinks);
        walker.max_depth(max_depth);

        let match_options = MatchOptions {
            case_sensitive: true,
            require_literal_separator: false,
            require_literal_leading_dot: !include_hidden,
        };

        let mut files = 0usize;
        let mut directories = 0usize;
        let mut extensions = BTreeMap::<String, usize>::new();
        let mut total_matches = 0usize;
        let mut matches = Vec::new();

        for entry in walker.build() {
            let Ok(entry) = entry else {
                // Skip unreadable entries instead of failing the whole search.
                continue;
            };

            let logical_path = entry.path();

            // Defensive check: when following symlinks, make sure the resolved path
            // still stays inside the workspace boundary.
            let Ok(resolved_path) = std::fs::canonicalize(logical_path) else {
                continue;
            };

            if !resolved_path.starts_with(&self.workspace_root) {
                continue;
            }

            let Ok(relative_to_search_root) = logical_path.strip_prefix(&search_root) else {
                continue;
            };

            let Ok(relative_to_workspace) = logical_path.strip_prefix(&self.workspace_root) else {
                continue;
            };

            let relative_to_search_root = normalize_path(relative_to_search_root);
            let relative_to_workspace = normalize_path(relative_to_workspace);

            if !matcher.matches_with(&relative_to_search_root, match_options) {
                continue;
            }

            let file_type = entry.file_type();
            let is_file = file_type
                .map(|ft| ft.is_file())
                .unwrap_or_else(|| logical_path.is_file());
            let is_dir = file_type
                .map(|ft| ft.is_dir())
                .unwrap_or_else(|| logical_path.is_dir());

            if !kind.matches(is_file, is_dir) {
                continue;
            }

            total_matches += 1;

            if is_file {
                files += 1;

                if let Some(extension) = logical_path
                    .extension()
                    .and_then(|v| v.to_str())
                    .filter(|v| !v.is_empty())
                {
                    *extensions.entry(extension.to_string()).or_insert(0) += 1;
                }
            }

            if is_dir {
                directories += 1;
            }

            if matches.len() < max_results {
                let extension = if is_file {
                    logical_path
                        .extension()
                        .and_then(|v| v.to_str())
                        .map(|v| v.to_string())
                } else {
                    None
                };

                matches.push(json!({
                    "path": relative_to_workspace,
                    "kind": if is_dir { "directory" } else { "file" },
                    "extension": extension
                }));
            }
        }

        let base_path = match search_root.strip_prefix(&self.workspace_root) {
            Ok(path) => normalize_path(path),
            Err(_) => ".".to_string(),
        };

        Ok(ToolExecutionResult::success(json!({
            "base_path": base_path,
            "pattern": pattern,
            "kind": kind.as_str(),
            "total_matches": total_matches,
            "returned_matches": matches.len(),
            "truncated": total_matches > matches.len(),
            "summary": {
                "files": files,
                "directories": directories,
                "extensions": extensions
            },
            "matches": matches
        })))
    }
}
