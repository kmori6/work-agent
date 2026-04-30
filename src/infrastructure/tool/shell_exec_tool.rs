use crate::domain::error::tool_error::ToolError;
use crate::domain::port::tool::{Tool, ToolExecutionPolicy, ToolOutput};
use crate::infrastructure::util::path::resolve_workspace_directory_path;
use crate::infrastructure::util::text::truncate_text;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::io::{Error, ErrorKind};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;
use tokio::time::{Duration, timeout};

const DEFAULT_TIMEOUT_SECS: u64 = 60;
const MAX_TIMEOUT_SECS: u64 = 600;
const MAX_OUTPUT_CHARS: usize = 32_000;

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommandLine {
    command: String,
    args: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
enum ShellRiskLevel {
    // Commands that stay within the workspace boundary. These are allowed to
    // run automatically so the agent can do routine local work.
    #[default]
    Workspace,
    // Commands that touch external systems, persistent state, or paths outside
    // the workspace. These require user confirmation.
    ExternalOrPersistent,
    // Commands that are outside the shell tool's safety boundary. These are
    // denied even if the user has generally allowed shell execution.
    OutOfBounds,
}

impl ShellRiskLevel {
    // Convert Commander-specific shell risk into the generic tool policy model.
    const fn policy(self) -> ToolExecutionPolicy {
        match self {
            Self::Workspace => ToolExecutionPolicy::Auto,
            Self::ExternalOrPersistent => ToolExecutionPolicy::Ask,
            Self::OutOfBounds => ToolExecutionPolicy::Forbidden,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct ShellRiskAssessment {
    level: ShellRiskLevel,
    reasons: Vec<&'static str>,
}

impl ShellRiskAssessment {
    fn raise(&mut self, level: ShellRiskLevel, reason: &'static str) {
        self.level = self.level.max(level);
        self.reasons.push(reason);
    }

    fn policy(&self) -> ToolExecutionPolicy {
        self.level.policy()
    }
}

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
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": format!(
                        "Optional timeout in seconds. Defaults to {DEFAULT_TIMEOUT_SECS}. Must be between 1 and {MAX_TIMEOUT_SECS}."
                    )
                },
            },
            "required": ["command"],
        })
    }

    fn execution_policy(&self, arguments: &Value) -> ToolExecutionPolicy {
        let Ok(command) = parse_command(arguments) else {
            return ToolExecutionPolicy::Ask;
        };

        assess_command_line(&command).policy()
    }

    async fn execute(&self, arguments: Value) -> Result<ToolOutput, ToolError> {
        let command = parse_command(&arguments)?;
        let workdir = parse_workdir(&arguments, &self.workspace_root)?;
        let timeout_secs = parse_timeout_secs(&arguments)?;

        ensure_command_line(&command)?;

        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());

        let output = timeout(
            Duration::from_secs(timeout_secs),
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
        .map_err(|err| map_io_error(err, &shell, &workdir, &command))?;

        let (stdout, _) = truncate_text(
            String::from_utf8_lossy(&output.stdout).to_string(),
            MAX_OUTPUT_CHARS,
        );
        let (stderr, _) = truncate_text(
            String::from_utf8_lossy(&output.stderr).to_string(),
            MAX_OUTPUT_CHARS,
        );
        let exit_code = output.status.code().unwrap_or(-1);

        Ok(ToolOutput::success(json!({
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

fn parse_timeout_secs(arguments: &Value) -> Result<u64, ToolError> {
    match arguments.get("timeout_secs") {
        None | Some(Value::Null) => Ok(DEFAULT_TIMEOUT_SECS),
        Some(value) => {
            let timeout_secs = value.as_u64().ok_or_else(|| {
                ToolError::InvalidArguments("'timeout_secs' must be an integer".into())
            })?;

            if timeout_secs == 0 {
                return Err(ToolError::InvalidArguments(
                    "'timeout_secs' must be greater than or equal to 1".into(),
                ));
            }

            if timeout_secs > MAX_TIMEOUT_SECS {
                return Err(ToolError::InvalidArguments(format!(
                    "'timeout_secs' must be less than or equal to {MAX_TIMEOUT_SECS}"
                )));
            }

            Ok(timeout_secs)
        }
    }
}

// shell command guardrails
fn assess_command_line(command: &str) -> ShellRiskAssessment {
    let mut assessment = ShellRiskAssessment::default();

    let Some(parsed) = parse_command_line(command) else {
        assessment.raise(
            ShellRiskLevel::OutOfBounds,
            "unparseable or non-simple shell command",
        );
        return assessment;
    };

    // forbidden: out of bounds
    if is_host_destructive_command(&parsed) {
        assessment.raise(ShellRiskLevel::OutOfBounds, "host destructive operation");
    }

    if is_privilege_escalation_command(&parsed) {
        assessment.raise(ShellRiskLevel::OutOfBounds, "privilege escalation");
    }

    if is_secret_access_command(&parsed) {
        assessment.raise(ShellRiskLevel::OutOfBounds, "secret access");
    }

    if !is_autonomous_allowed_command(&parsed) {
        assessment.raise(
            ShellRiskLevel::ExternalOrPersistent,
            "not a simple workspace-local command",
        );
    }

    assessment
}

fn ensure_command_line(command: &str) -> Result<(), ToolError> {
    let assessment = assess_command_line(command);

    if assessment.policy() == ToolExecutionPolicy::Forbidden {
        return Err(ToolError::PermissionDenied(format!(
            "command is forbidden by shell_exec policy: {}",
            assessment.reasons.join(", ")
        )));
    }

    Ok(())
}

fn parse_command_line(command: &str) -> Option<CommandLine> {
    // Parse the input as bash, then accept only one plain command.
    // Example: `rg TODO src` is accepted; `rg TODO src | head` is rejected.
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_bash::LANGUAGE.into())
        .ok()?;

    let tree = parser.parse(command, None)?;
    let root = tree.root_node();

    // Syntax errors mean we cannot safely reason about the command.
    // Example: an unfinished quote like `echo "hello` is rejected.
    if root.has_error() {
        return None;
    }

    // Require exactly one top-level statement. Chained commands are harder to
    // classify safely, so `pwd && ls` and `echo hi; rm file` are rejected.
    let mut cursor = root.walk();
    let stmts: Vec<_> = root.named_children(&mut cursor).collect();
    let [stmt] = stmts.as_slice() else {
        return None;
    };

    // The statement must be a simple command node. Bash compounds such as
    // pipelines, subshells, loops, redirects, and assignments are rejected.
    if stmt.kind() != "command" {
        return None;
    }

    let bytes = command.as_bytes();
    let mut child_cursor = stmt.walk();
    let mut children = stmt.named_children(&mut child_cursor);

    // The first named child is the executable name.
    // Example: for `git status --short`, this extracts `git`.
    let name_node = children.next()?;
    if name_node.kind() != "command_name" {
        return None;
    }

    let command = name_node.utf8_text(bytes).ok()?.to_ascii_lowercase();

    // Every argument must be a plain word. Quoted strings, variables, globs,
    // and command substitutions are rejected, e.g. `echo "$HOME"` or `cat *.rs`.
    let mut args = Vec::new();
    for node in children {
        if node.kind() != "word" {
            return None;
        }

        args.push(node.utf8_text(bytes).ok()?.to_ascii_lowercase());
    }

    Some(CommandLine { command, args })
}

// 1. forbidden patterns
fn is_host_destructive_command(command_line: &CommandLine) -> bool {
    let command = command_line
        .command
        .rsplit('/')
        .next()
        .unwrap_or(&command_line.command);

    match command {
        // System power control.
        "shutdown" | "reboot" | "poweroff" | "halt" => true,

        // Filesystem creation or formatting.
        "mkfs" => true,

        program if program.starts_with("mkfs.") => true,

        // Block device wiping or discard operations.
        "wipefs" | "blkdiscard" => true,

        // Service-manager power control.
        "systemctl" => command_line
            .args
            .iter()
            .any(|arg| matches!(arg.as_str(), "reboot" | "poweroff" | "halt")),

        // Raw writes to host-critical targets.
        "dd" => command_line
            .args
            .iter()
            .any(|arg| arg.strip_prefix("of=").is_some_and(is_host_critical_path)),

        // Removal of host-critical paths.
        "rm" => command_line
            .args
            .iter()
            .any(|arg| is_host_critical_path(arg)),

        // Recursive permission or ownership changes on host-critical paths.
        "chmod" | "chown" => {
            has_recursive_flag(&command_line.args)
                && command_line
                    .args
                    .iter()
                    .any(|arg| is_host_critical_path(arg))
        }

        _ => false,
    }
}

fn is_privilege_escalation_command(command_line: &CommandLine) -> bool {
    let command = command_line
        .command
        .rsplit('/')
        .next()
        .unwrap_or(&command_line.command);

    matches!(
        command,
        // Run another command as a different user, often root.
        "sudo" | "doas" | "pkexec"
        // Switch user shell/session.
        | "su"
        // Edit files through sudo privileges.
        | "sudoedit"
    )
}

fn is_secret_access_command(command_line: &CommandLine) -> bool {
    command_line
        .args
        .iter()
        .any(|arg| is_secret_access_path(arg))
}

// 2. allowed patterns
fn is_autonomous_allowed_command(command_line: &CommandLine) -> bool {
    if command_line.command.contains('/') {
        return false;
    }

    let command_is_allowed = match command_line.command.as_str() {
        "pwd" => command_line.args.is_empty(),

        "ls" | "rg" | "grep" | "cat" | "head" | "tail" | "wc" | "du" | "mkdir" | "touch" | "cp"
        | "mv" | "rm" => true,

        "sed" => !command_line
            .args
            .iter()
            .any(|arg| arg == "-i" || arg.starts_with("-i")),

        "find" => !command_line
            .args
            .iter()
            .any(|arg| matches!(arg.as_str(), "-exec" | "-execdir" | "-delete" | "-ok")),

        "git" => matches!(
            command_line.args.first().map(String::as_str),
            Some("status" | "diff" | "log" | "show" | "rev-parse" | "ls-files")
        ),

        _ => false,
    };

    if !command_is_allowed {
        return false;
    }

    command_line.args.iter().all(|arg| {
        if arg.is_empty() {
            return false;
        }

        if arg.starts_with('-') && !arg.contains('/') && !arg.contains("://") {
            return true;
        }

        !arg.starts_with('/')
            && !arg.starts_with('~')
            && !arg.split('/').any(|seg| seg == "..")
            && !arg.contains("://")
            && !arg.starts_with("git@")
            && !arg.chars().any(|c| matches!(c, '*' | '?' | '[' | ']'))
    })
}

// helpers
fn map_io_error(err: Error, shell: &str, workdir: &Path, command: &str) -> ToolError {
    match err.kind() {
        ErrorKind::NotFound => ToolError::Unavailable(format!("shell not found: {shell}")),
        ErrorKind::PermissionDenied => ToolError::PermissionDenied(format!(
            "permission denied while executing command in {}: {}",
            workdir.display(),
            command
        )),
        _ => ToolError::ExecutionFailed(format!(
            "failed to execute command in {} via {}: {} ({err})",
            workdir.display(),
            shell,
            command
        )),
    }
}

fn has_recursive_flag(args: &[String]) -> bool {
    args.iter().any(|arg| {
        arg == "-r"
            || arg == "-R"
            || (arg.starts_with('-') && arg.chars().any(|c| matches!(c, 'r' | 'R')))
    })
}

fn is_host_critical_path(path: &str) -> bool {
    path == "/"
        || path == "/*"
        || path == "/bin"
        || path.starts_with("/bin/")
        || path == "/boot"
        || path.starts_with("/boot/")
        || path == "/dev"
        || path.starts_with("/dev/")
        || path == "/etc"
        || path.starts_with("/etc/")
        || path == "/lib"
        || path.starts_with("/lib/")
        || path == "/lib64"
        || path.starts_with("/lib64/")
        || path == "/proc"
        || path.starts_with("/proc/")
        || path == "/root"
        || path.starts_with("/root/")
        || path == "/sbin"
        || path.starts_with("/sbin/")
        || path == "/sys"
        || path.starts_with("/sys/")
        || path == "/usr"
        || path.starts_with("/usr/")
        || path == "/var"
        || path.starts_with("/var/")
        || path == "/home"
        || path.starts_with("/home/")
        || path == "/opt"
        || path.starts_with("/opt/")
        || path == "/mnt"
        || path.starts_with("/mnt/")
        || path == "/media"
        || path.starts_with("/media/")
        || path == "/run"
        || path.starts_with("/run/")
        || path == "/tmp"
        || path.starts_with("/tmp/")
}

fn is_secret_access_path(path: &str) -> bool {
    if is_safe_secret_example_path(path) {
        return false;
    }

    let normalized = path.trim_start_matches("./");

    normalized == ".env"
        || normalized.starts_with(".env.")
        || normalized.contains("/.env")
        || normalized.contains("/.env.")
        || normalized == ".ssh"
        || normalized.starts_with(".ssh/")
        || normalized.contains("/.ssh/")
        || normalized == "id_rsa"
        || normalized.ends_with("/id_rsa")
        || normalized == "id_ed25519"
        || normalized.ends_with("/id_ed25519")
        || normalized == ".aws/credentials"
        || normalized.ends_with("/.aws/credentials")
        || normalized == ".gnupg"
        || normalized.starts_with(".gnupg/")
        || normalized.contains("/.gnupg/")
        || normalized == ".netrc"
        || normalized.ends_with("/.netrc")
        || normalized == ".npmrc"
        || normalized.ends_with("/.npmrc")
        || normalized == ".pypirc"
        || normalized.ends_with("/.pypirc")
        || normalized == ".docker/config.json"
        || normalized.ends_with("/.docker/config.json")
        || normalized == ".kube/config"
        || normalized.ends_with("/.kube/config")
        || normalized.ends_with(".pem")
        || normalized.ends_with(".key")
}

fn is_safe_secret_example_path(path: &str) -> bool {
    let normalized = path.trim_start_matches("./");
    let file_name = normalized.rsplit('/').next().unwrap_or(normalized);

    matches!(
        file_name,
        ".env.example" | ".env.sample" | ".env.template" | "example.env"
    )
}
