use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;

use crate::domain::repository::chat_message_repository::ChatMessageRepository;
use crate::domain::repository::chat_session_repository::ChatSessionRepository;
use crate::presentation::state::app_state::AppState;

const DEFAULT_LIMIT: usize = 20;
const MAX_LIMIT: usize = 100;
const TITLE_MAX_CHARS: usize = 40;

#[derive(Debug, Deserialize)]
pub struct GetSessionsQuery {
    pub limit: Option<usize>,
}

pub async fn get_session_handler(
    State(state): State<AppState>,
    Query(query): Query<GetSessionsQuery>,
) -> impl IntoResponse {
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);

    let sessions = match state.chat_session_repository.list_recent(limit).await {
        Ok(sessions) => sessions,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": {
                        "code": "failed_to_get_sessions",
                        "message": err.to_string(),
                    }
                })),
            );
        }
    };

    let session_ids = sessions
        .iter()
        .map(|session| session.id)
        .collect::<Vec<_>>();

    let message_summaries = match state
        .chat_message_repository
        .summarize_by_session_ids(&session_ids)
        .await
    {
        Ok(summaries) => summaries,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": {
                        "code": "failed_to_get_sessions",
                        "message": err.to_string(),
                    }
                })),
            );
        }
    };

    let message_summaries = message_summaries
        .into_iter()
        .map(|summary| (summary.session_id, summary))
        .collect::<HashMap<_, _>>();

    let sessions = sessions
        .into_iter()
        .map(|session| {
            let message_summary = message_summaries.get(&session.id);

            json!({
                "id": session.id.to_string(),
                "title": title_from_first_user_message(
                    message_summary.and_then(|summary| summary.first_user_message.as_deref()), TITLE_MAX_CHARS
                ),
                "status": session.status.as_str(),
                "created_at": session.created_at.to_rfc3339(),
                "updated_at": session.updated_at.to_rfc3339(),
                "message_count": message_summary
                    .map(|summary| summary.message_count)
                    .unwrap_or(0),
            })
        })
        .collect::<Vec<_>>();

    (
        StatusCode::OK,
        Json(json!({
            "sessions": sessions,
        })),
    )
}

fn title_from_first_user_message(text: Option<&str>, max_chars: usize) -> String {
    let Some(text) = text else {
        return "Untitled session".to_string();
    };

    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let title = normalized.chars().take(max_chars).collect::<String>();

    if title.is_empty() {
        "Untitled session".to_string()
    } else {
        title
    }
}
