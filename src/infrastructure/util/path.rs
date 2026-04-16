use crate::domain::error::tool_error::ToolError;
use std::path::{Component, Path, PathBuf};

pub fn has_parent_traversal(path: &Path) -> bool {
    path.components()
        .any(|component| component == Component::ParentDir)
}

pub fn normalize_path(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    if text.is_empty() {
        ".".to_string()
    } else {
        text
    }
}

pub fn resolve_workspace_file_path(
    workspace_root: &Path,
    input_path: &str,
) -> Result<PathBuf, ToolError> {
    let input_path = input_path.trim();
    if input_path.is_empty() {
        return Err(ToolError::InvalidArguments(
            "'path' must not be empty".into(),
        ));
    }

    let input_path = Path::new(input_path);

    if !input_path.is_absolute() && has_parent_traversal(input_path) {
        return Err(ToolError::PermissionDenied(
            "path traversal is not allowed in relative paths".into(),
        ));
    }

    let candidate = if input_path.is_absolute() {
        input_path.to_path_buf()
    } else {
        workspace_root.join(input_path)
    };

    let resolved = std::fs::canonicalize(&candidate)
        .map_err(|err| ToolError::ExecutionFailed(format!("failed to resolve file path: {err}")))?;

    if !resolved.starts_with(workspace_root) {
        return Err(ToolError::PermissionDenied(
            "'path' resolved outside the workspace".into(),
        ));
    }

    if !resolved.is_file() {
        return Err(ToolError::InvalidArguments(
            "'path' must point to a file".into(),
        ));
    }

    Ok(resolved)
}

pub fn resolve_workspace_directory_path(
    workspace_root: &Path,
    input_path: &str,
) -> Result<PathBuf, ToolError> {
    let input_path = input_path.trim();
    if input_path.is_empty() {
        return Err(ToolError::InvalidArguments(
            "'base_path' must not be empty".into(),
        ));
    }

    let input_path = Path::new(input_path);

    if !input_path.is_absolute() && has_parent_traversal(input_path) {
        return Err(ToolError::PermissionDenied(
            "path traversal is not allowed in relative paths".into(),
        ));
    }

    let candidate = if input_path.is_absolute() {
        input_path.to_path_buf()
    } else {
        workspace_root.join(input_path)
    };

    let resolved = std::fs::canonicalize(&candidate).map_err(|err| {
        ToolError::ExecutionFailed(format!("failed to resolve directory path: {err}"))
    })?;

    if !resolved.starts_with(workspace_root) {
        return Err(ToolError::PermissionDenied(
            "'base_path' resolved outside the workspace".into(),
        ));
    }

    if !resolved.is_dir() {
        return Err(ToolError::InvalidArguments(
            "'base_path' must point to a directory".into(),
        ));
    }

    Ok(resolved)
}
