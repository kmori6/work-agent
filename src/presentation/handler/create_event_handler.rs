use crate::domain::model::chat_session_event::ChatSessionEvent;
use crate::domain::model::tool_approval::ToolApprovalResponse;
use crate::domain::model::tool_call::ToolCallOutputStatus;
use crate::presentation::state::app_state::AppState;
use axum::{
    extract::State,
    response::{
        IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures::stream;
use serde_json::json;
use std::{convert::Infallible, time::Duration};
use tokio::sync::broadcast;

pub async fn create_event_handler(State(state): State<AppState>) -> impl IntoResponse {
    let rx = state.event_service.subscribe();

    let stream = stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(event) => return Some((Ok::<Event, Infallible>(to_sse_event(event)), rx)),
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}

fn to_sse_event(event: ChatSessionEvent) -> Event {
    match event {
        ChatSessionEvent::AgentTurnStarted { session_id } => Event::default()
            .event("agent_turn_started")
            .data(json!({ "session_id": session_id }).to_string()),

        ChatSessionEvent::LlmStarted { session_id } => Event::default()
            .event("llm_started")
            .data(json!({ "session_id": session_id }).to_string()),

        ChatSessionEvent::LlmFinished { session_id } => Event::default()
            .event("llm_finished")
            .data(json!({ "session_id": session_id }).to_string()),

        ChatSessionEvent::ToolCallStarted {
            session_id,
            call_id,
            tool_name,
            arguments,
        } => Event::default().event("tool_call_started").data(
            json!({
                "session_id": session_id,
                "call_id": call_id,
                "tool_name": tool_name,
                "arguments": arguments,
            })
            .to_string(),
        ),

        ChatSessionEvent::ToolCallFinished {
            session_id,
            call_id,
            tool_name,
            output,
            status,
        } => Event::default().event("tool_call_finished").data(
            json!({
                "session_id": session_id,
                "call_id": call_id,
                "tool_name": tool_name,
                "output": output,
                "status": tool_call_output_status_as_str(status),
            })
            .to_string(),
        ),

        ChatSessionEvent::AssistantMessageCreated {
            session_id,
            message_id,
            content,
        } => Event::default().event("assistant_message_created").data(
            json!({
                "session_id": session_id,
                "message_id": message_id,
                "content": content,
            })
            .to_string(),
        ),

        ChatSessionEvent::ToolCallApprovalRequested {
            session_id,
            call_id,
            tool_name,
            arguments,
            policy,
        } => Event::default().event("tool_call_approval_requested").data(
            json!({
                "session_id": session_id,
                "call_id": call_id,
                "tool_name": tool_name,
                "arguments": arguments,
                "policy": policy.as_str(),
            })
            .to_string(),
        ),

        ChatSessionEvent::ToolCallApprovalResolved {
            session_id,
            call_id,
            tool_name,
            decision,
        } => Event::default().event("tool_call_approval_resolved").data(
            json!({
                "session_id": session_id,
                "call_id": call_id,
                "tool_name": tool_name,
                "decision": tool_approval_decision_as_str(decision),
            })
            .to_string(),
        ),

        ChatSessionEvent::AgentTurnCompleted { session_id } => Event::default()
            .event("agent_turn_completed")
            .data(json!({ "session_id": session_id }).to_string()),

        ChatSessionEvent::AgentTurnFailed {
            session_id,
            message,
        } => Event::default().event("agent_turn_failed").data(
            json!({
                "session_id": session_id,
                "message": message,
            })
            .to_string(),
        ),
    }
}

fn tool_call_output_status_as_str(status: ToolCallOutputStatus) -> &'static str {
    match status {
        ToolCallOutputStatus::Success => "success",
        ToolCallOutputStatus::Error => "error",
    }
}

fn tool_approval_decision_as_str(decision: ToolApprovalResponse) -> &'static str {
    match decision {
        ToolApprovalResponse::Approved => "approved",
        ToolApprovalResponse::Denied => "denied",
    }
}
