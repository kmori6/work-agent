use thiserror::Error;

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("unknown tool: {0}")]
    UnknownTool(String),

    #[error("invalid tool arguments: {0}")]
    InvalidArguments(String),

    #[error("tool permission denied: {0}")]
    PermissionDenied(String),

    #[error("tool is unavailable: {0}")]
    Unavailable(String),

    #[error("tool execution timed out")]
    Timeout,

    #[error("tool execution failed: {0}")]
    ExecutionFailed(String),
}
