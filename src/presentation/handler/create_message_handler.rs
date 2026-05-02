use crate::domain::model::chat_session_event::ChatSessionEvent;
use crate::domain::model::message::Message;
use crate::presentation::state::app_state::AppState;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::mpsc;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct CreateMessageRequest {
    pub user_message: Message,
}

pub async fn create_message_handler(
    State(state): State<AppState>,
    Path(session_id): Path<Uuid>,
    Json(request): Json<CreateMessageRequest>,
) -> Response {
    let agent_usecase = state.agent_usecase.clone();
    let event_service = state.event_service.clone();

    let saved = agent_usecase
        .submit_user_message(session_id, request.user_message)
        .await;

    let saved = match saved {
        Ok(saved) => saved,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": {
                        "code": "failed_to_create_message",
                        "message": err.to_string(),
                    }
                })),
            )
                .into_response();
        }
    };

    let response = json!({
        "session_id": saved.session_id.to_string(),
        "message_id": saved.id.to_string(),
        "created_at": saved.created_at.to_rfc3339(),
    });

    let start_message = saved.clone();
    tokio::spawn(async move {
        let (event_tx, mut event_rx) = mpsc::channel::<ChatSessionEvent>(32);

        let publisher_event_service = event_service.clone();
        let event_publisher = tokio::spawn(async move {
            while let Some(event) = event_rx.recv().await {
                publisher_event_service.publish(event);
            }
        });

        if let Err(err) = agent_usecase
            .start_turn(session_id, start_message, event_tx)
            .await
        {
            log::warn!("failed to start turn for session {session_id}: {err}");

            event_service.publish(ChatSessionEvent::AgentTurnFailed {
                session_id,
                message: err.to_string(),
            });
        }

        if let Err(err) = event_publisher.await {
            log::warn!("failed to publish agent events for session {session_id}: {err}");
        }
    });

    (StatusCode::ACCEPTED, Json(response)).into_response()
}
