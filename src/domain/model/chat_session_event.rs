use crate::domain::model::tool_approval::ToolApprovalResponse;
use crate::domain::model::tool_call::ToolCallOutputStatus;
use crate::domain::model::tool_execution_policy::ToolExecutionPolicy;
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub enum ChatSessionEvent {
    AgentTurnStarted {
        session_id: Uuid,
    },
    LlmStarted {
        session_id: Uuid,
    },
    LlmFinished {
        session_id: Uuid,
    },
    ToolCallStarted {
        session_id: Uuid,
        call_id: String,
        tool_name: String,
        arguments: Value,
    },
    ToolCallFinished {
        session_id: Uuid,
        call_id: String,
        tool_name: String,
        output: Value,
        status: ToolCallOutputStatus,
    },
    AssistantMessageCreated {
        session_id: Uuid,
        message_id: Uuid,
        content: String,
    },
    ToolCallApprovalRequested {
        session_id: Uuid,
        call_id: String,
        tool_name: String,
        arguments: Value,
        policy: ToolExecutionPolicy,
    },
    ToolCallApprovalResolved {
        session_id: Uuid,
        call_id: String,
        tool_name: String,
        decision: ToolApprovalResponse,
    },
    AgentTurnCompleted {
        session_id: Uuid,
    },
    AgentTurnFailed {
        session_id: Uuid,
        message: String,
    },
}
