/// A tool's default execution policy for a specific invocation.
///
/// This is declared by the tool implementation and then combined with
/// persisted `tool_execution_rules` to produce the final execution decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolExecutionPolicy {
    /// Execute automatically unless a persisted rule asks or denies it.
    Auto,
    /// Ask the user before execution unless a persisted rule allows or denies it.
    Ask,
    /// Always ask before execution unless a persisted rule denies it.
    ///
    /// A stored `allow` rule must not bypass this policy.
    ConfirmEveryTime,
}

impl ToolExecutionPolicy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Ask => "ask",
            Self::ConfirmEveryTime => "confirm",
        }
    }
}
