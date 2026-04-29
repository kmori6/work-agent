use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;
use uuid::Uuid;

use crate::domain::repository::chat_session_repository::ChatSessionRepository;
use crate::presentation::state::app_state::AppState;

pub async fn get_session_handler(
    State(state): State<AppState>,
    Path(session_id): Path<Uuid>,
) -> Response {
    match state.chat_session_repository.find_by_id(session_id).await {
        Ok(Some(session)) => (
            StatusCode::OK,
            Json(json!({
                "id": session.id.to_string(),
                "status": session.status.as_str(),
                "created_at": session.created_at.to_rfc3339(),
                "updated_at": session.updated_at.to_rfc3339(),
            })),
        )
            .into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": {
                    "code": "session_not_found",
                    "message": format!("session not found: {session_id}"),
                }
            })),
        )
            .into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": {
                    "code": "failed_to_get_session",
                    "message": err.to_string(),
                }
            })),
        )
            .into_response(),
    }
}
