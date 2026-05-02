use std::sync::Arc;

use crate::application::usecase::agent_usecase::AgentUsecase;
use crate::domain::service::event_service::EventService;
use crate::infrastructure::llm::bedrock_llm_provider::BedrockLlmProvider;
use crate::infrastructure::persistence::postgres_awaiting_tool_approval_repository::PostgresAwaitingToolApprovalRepository;
use crate::infrastructure::persistence::postgres_chat_message_repository::PostgresChatMessageRepository;
use crate::infrastructure::persistence::postgres_chat_session_repository::PostgresChatSessionRepository;
use crate::infrastructure::persistence::postgres_token_usage_repository::PostgresTokenUsageRepository;
use crate::infrastructure::persistence::postgres_tool_approval_repository::PostgresToolApprovalRepository;

#[derive(Clone)]
pub struct AppState {
    pub chat_session_repository: PostgresChatSessionRepository,
    pub chat_message_repository: PostgresChatMessageRepository,
    pub event_service: Arc<EventService>,
    pub agent_usecase: Arc<
        AgentUsecase<
            BedrockLlmProvider,
            PostgresChatSessionRepository,
            PostgresChatMessageRepository,
            PostgresTokenUsageRepository,
            PostgresToolApprovalRepository,
            PostgresAwaitingToolApprovalRepository,
        >,
    >,
}
