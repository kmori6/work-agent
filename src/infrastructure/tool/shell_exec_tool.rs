use crate::domain::error::tool_error::ToolError;
use crate::domain::model::tool::ToolExecutionResult;
use crate::domain::port::tool::{Tool, ToolExecutionPolicy};
use crate::infrastructure::util::path::resolve_workspace_directory_path;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;
use tokio::time::Duration;
use tokio::time::timeout;

const TIMEOUT_SECS: u64 = 60;
const MAX_OUTPUT_CHARS: usize = 32_000;

pub struct ShellExecTool {
    workspace_root: PathBuf,
}

impl ShellExecTool {
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
impl Tool for ShellExecTool {
    fn name(&self) -> &str {
        "shell_exec"
    }

    fn description(&self) -> &str {
        "Execute one non-interactive command in the workspace. The command runs in the workspace root by default. Do not use `cd /workspace && ...`; use `workdir` to run in a subdirectory."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Command to run in the workspace or in `workdir`. Do not prefix with `cd ... &&`. Example: `cargo check`."
                },
                "workdir": {
                    "type": "string",
                    "description": "Optional subdirectory inside the workspace. Use this instead of `cd`. Example: use `{ \"command\": \"cargo test\", \"workdir\": \"crates/api\" }` instead of `cd crates/api && cargo test`."
                }
            },
            "required": ["command"],
        })
    }

    fn execution_policy(&self, _arguments: &Value) -> ToolExecutionPolicy {
        ToolExecutionPolicy::Ask
    }

    async fn execute(&self, arguments: Value) -> Result<ToolExecutionResult, ToolError> {
        let command = parse_command(&arguments)?;
        let workdir = parse_workdir(&arguments, &self.workspace_root)?;

        validate_command(&command)?;

        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let command_preview = preview_command(&command);

        let output = timeout(
            Duration::from_secs(TIMEOUT_SECS),
            Command::new(&shell)
                .arg("-lc")
                .arg(&command)
                .current_dir(&workdir)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .output(),
        )
        .await
        .map_err(|_| ToolError::Timeout)?
        .map_err(|err| map_io_error(err, &shell, &workdir, &command_preview))?;

        let stdout = truncate_output(&String::from_utf8_lossy(&output.stdout), MAX_OUTPUT_CHARS);
        let stderr = truncate_output(&String::from_utf8_lossy(&output.stderr), MAX_OUTPUT_CHARS);
        let exit_code = output.status.code().unwrap_or(-1);

        Ok(ToolExecutionResult::success(json!({
            "stdout": stdout,
            "stderr": stderr,
            "exit_code": exit_code
        })))
    }
}

fn parse_command(arguments: &Value) -> Result<String, ToolError> {
    let command = arguments
        .get("command")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ToolError::InvalidArguments("missing or invalid 'command'".into()))?;

    Ok(command.to_string())
}

fn parse_workdir(arguments: &Value, workspace_root: &Path) -> Result<PathBuf, ToolError> {
    match arguments.get("workdir") {
        Some(value) => {
            let workdir = value
                .as_str()
                .ok_or_else(|| ToolError::InvalidArguments("'workdir' must be a string".into()))?;
            resolve_workspace_directory_path(workspace_root, workdir)
        }
        None => Ok(workspace_root.to_path_buf()),
    }
}

fn validate_command(command: &str) -> Result<(), ToolError> {
    let lowered = command.to_ascii_lowercase();

    // HACK: simple blacklist to prevent obviously dangerous commands. This is not a security boundary, just a best effort to catch common mistakes.
    let denied = [
        "sudo ", " ssh ", "scp ", "curl ", "wget ", "rm ", "mv ", "cp ", "chmod ", "chown ",
        "nohup ", "tmux ", "screen ", "shutdown", "reboot", "mkfs", " dd ", ">", ">>", "tee ",
        "sed -i", "&",
    ];

    if denied.iter().any(|token| lowered.contains(token)) {
        return Err(ToolError::PermissionDenied(
            "command is not allowed by shell_exec policy".into(),
        ));
    }

    Ok(())
}

fn preview_command(command: &str) -> String {
    const MAX_LEN: usize = 120;
    let trimmed = command.trim();

    if trimmed.chars().count() <= MAX_LEN {
        trimmed.to_string()
    } else {
        let prefix: String = trimmed.chars().take(MAX_LEN).collect();
        format!("{prefix}...")
    }
}

fn map_io_error(
    err: std::io::Error,
    shell: &str,
    workdir: &Path,
    command_preview: &str,
) -> ToolError {
    match err.kind() {
        ErrorKind::NotFound => ToolError::Unavailable(format!("shell not found: {shell}")),
        ErrorKind::PermissionDenied => ToolError::PermissionDenied(format!(
            "permission denied while executing command in {}: {}",
            workdir.display(),
            command_preview
        )),
        _ => ToolError::ExecutionFailed(format!(
            "failed to execute command in {} via {}: {} ({err})",
            workdir.display(),
            shell,
            command_preview
        )),
    }
}

fn truncate_output(output: &str, max_chars: usize) -> String {
    if output.chars().count() <= max_chars {
        return output.to_string();
    }

    const NOTICE: &str = "\n...[truncated]";
    let notice_len = NOTICE.chars().count();

    let keep_len = max_chars - notice_len;
    let prefix = output.chars().take(keep_len).collect::<String>();

    format!("{prefix}{NOTICE}")
}
