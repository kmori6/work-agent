// src/domain/model/tool_approval.rs
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct ToolApproval {
    pub session_id: Uuid,
    pub tool_call_id: String,
    pub tool_name: String,
    pub arguments: Value,
    pub decision: ToolApprovalDecision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolApprovalDecision {
    Approved,
    Denied,
}

impl ToolApprovalDecision {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::Denied => "denied",
        }
    }
}
