use crate::domain::model::chat_session_event::ChatSessionEvent;
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
pub struct ResolveApprovalRequest {
    pub decision: ApprovalDecisionRequest,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecisionRequest {
    Approved,
    Denied,
}

impl ApprovalDecisionRequest {
    const fn as_str(&self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::Denied => "denied",
        }
    }
}

pub async fn resolve_approval_handler(
    State(state): State<AppState>,
    Path(session_id): Path<Uuid>,
    Json(request): Json<ResolveApprovalRequest>,
) -> Response {
    let decision = request.decision;
    let decision_text = decision.as_str();

    let agent_usecase = state.agent_usecase.clone();
    let event_service = state.event_service.clone();

    tokio::spawn(async move {
        let (event_tx, mut event_rx) = mpsc::channel::<ChatSessionEvent>(32);

        let publisher_event_service = event_service.clone();
        let event_publisher = tokio::spawn(async move {
            while let Some(event) = event_rx.recv().await {
                publisher_event_service.publish(event);
            }
        });

        let result = match decision {
            ApprovalDecisionRequest::Approved => {
                agent_usecase.approve_approval(session_id, event_tx).await
            }
            ApprovalDecisionRequest::Denied => {
                agent_usecase.deny_approval(session_id, event_tx).await
            }
        };

        if let Err(err) = result {
            log::warn!("failed to resolve approval for session {session_id}: {err}");

            event_service.publish(ChatSessionEvent::AgentTurnFailed {
                session_id,
                message: err.to_string(),
            });
        }

        if let Err(err) = event_publisher.await {
            log::warn!("failed to publish approval events for session {session_id}: {err}");
        }
    });

    (
        StatusCode::ACCEPTED,
        Json(json!({
            "session_id": session_id.to_string(),
            "decision": decision_text,
        })),
    )
        .into_response()
}
