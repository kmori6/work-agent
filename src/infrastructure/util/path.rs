use crate::domain::error::tool_error::ToolError;
use std::path::{Component, Path, PathBuf};

/// Check if the path contains any parent directory components (".."), which can be used for path traversal attacks.
pub fn contains_parent_dir(path: &Path) -> bool {
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

    // security gardrail: disallow path traversal in relative paths
    if !input_path.is_absolute() && contains_parent_dir(input_path) {
        return Err(ToolError::PermissionDenied(
            "path traversal is not allowed in relative paths".into(),
        ));
    }

    let resolved_path = if input_path.is_absolute() {
        input_path.to_path_buf()
    } else {
        workspace_root.join(input_path)
    };

    let canonical_path = std::fs::canonicalize(&resolved_path)
        .map_err(|err| ToolError::ExecutionFailed(format!("failed to resolve file path: {err}")))?;

    if !canonical_path.starts_with(workspace_root) {
        return Err(ToolError::PermissionDenied(
            "'path' canonicalized outside the workspace".into(),
        ));
    }

    if !canonical_path.is_file() {
        return Err(ToolError::InvalidArguments(
            "'path' must point to a file".into(),
        ));
    }

    Ok(canonical_path)
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

    if !input_path.is_absolute() && contains_parent_dir(input_path) {
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

/// Resolve a file path within the workspace, then read it as UTF-8 text
/// with size and binary-safety checks.
///
/// Returns the canonical path and the file content.
pub async fn read_workspace_text_file(
    workspace_root: &Path,
    path: &str,
    max_file_size: u64,
) -> Result<(PathBuf, String), ToolError> {
    let resolved = resolve_workspace_file_path(workspace_root, path)?;

    let metadata = tokio::fs::metadata(&resolved).await.map_err(|err| {
        ToolError::ExecutionFailed(format!("failed to read file metadata: {err}"))
    })?;

    if metadata.len() > max_file_size {
        return Err(ToolError::ExecutionFailed(format!(
            "file is too large: {} bytes (max: {})",
            metadata.len(),
            max_file_size
        )));
    }

    let bytes = tokio::fs::read(&resolved)
        .await
        .map_err(|err| ToolError::ExecutionFailed(format!("failed to read file: {err}")))?;

    if bytes.contains(&0) {
        return Err(ToolError::ExecutionFailed(
            "file appears to be binary".into(),
        ));
    }

    let content = String::from_utf8(bytes)
        .map_err(|_| ToolError::ExecutionFailed("file is not valid UTF-8 text".into()))?;

    Ok((resolved, content))
}
