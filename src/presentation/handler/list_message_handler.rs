use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::domain::model::chat_message::ChatMessage;
use crate::domain::model::message::MessageContent;
use crate::domain::model::role::Role;
use crate::domain::model::tool_call::ToolCallOutputStatus;
use crate::domain::repository::chat_message_repository::ChatMessageRepository;
use crate::domain::repository::chat_session_repository::ChatSessionRepository;
use crate::presentation::state::app_state::AppState;

#[derive(Debug, Deserialize)]
pub struct ListMessageQuery {
    pub limit: Option<usize>,
}

pub async fn list_message_handler(
    State(state): State<AppState>,
    Path(session_id): Path<Uuid>,
    Query(query): Query<ListMessageQuery>,
) -> Response {
    match state.chat_session_repository.find_by_id(session_id).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({
                    "error": {
                        "code": "session_not_found",
                        "message": format!("session not found: {session_id}"),
                    }
                })),
            )
                .into_response();
        }
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": {
                        "code": "failed_to_get_session_messages",
                        "message": err.to_string(),
                    }
                })),
            )
                .into_response();
        }
    }

    let messages = match state
        .chat_message_repository
        .list_for_session(session_id)
        .await
    {
        Ok(messages) => messages,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": {
                        "code": "failed_to_get_session_messages",
                        "message": err.to_string(),
                    }
                })),
            )
                .into_response();
        }
    };

    let messages = apply_limit(messages, query.limit)
        .into_iter()
        .map(message_to_json)
        .collect::<Vec<_>>();

    (
        StatusCode::OK,
        Json(json!({
            "session_id": session_id.to_string(),
            "messages": messages,
        })),
    )
        .into_response()
}

fn role_as_str(role: Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
    }
}

fn message_to_json(chat_message: ChatMessage) -> Value {
    json!({
        "id": chat_message.id.to_string(),
        "role": role_as_str(chat_message.message.role),
        "contents": chat_message
            .message
            .contents
            .iter()
            .filter_map(content_to_json)
            .collect::<Vec<_>>(),
        "created_at": chat_message.created_at.to_rfc3339(),
    })
}

fn content_to_json(content: &MessageContent) -> Option<Value> {
    match content {
        MessageContent::InputText(text) => Some(json!({
            "type": "input_text",
            "text": text,
        })),
        MessageContent::OutputText(text) => Some(json!({
            "type": "output_text",
            "text": text,
        })),
        MessageContent::ToolCall(call) => Some(json!({
            "type": "tool_call",
            "call_id": call.call_id,
            "tool_name": call.name,
            "arguments": call.arguments,
        })),
        MessageContent::ToolCallOutput(output) => Some(json!({
            "type": "tool_call_output",
            "call_id": output.call_id,
            "output": output.output,
            "result_status": match output.status {
                ToolCallOutputStatus::Success => "success",
                ToolCallOutputStatus::Error => "error",
            },
        })),
        MessageContent::InputImage(_) | MessageContent::InputFile(_) => None,
    }
}

fn apply_limit(messages: Vec<ChatMessage>, limit: Option<usize>) -> Vec<ChatMessage> {
    let Some(limit) = limit else {
        return messages;
    };

    let skip = messages.len().saturating_sub(limit);
    messages.into_iter().skip(skip).collect()
}
