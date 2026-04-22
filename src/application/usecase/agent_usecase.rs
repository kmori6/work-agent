use crate::application::error::agent_usecase_error::AgentUsecaseError;
use crate::domain::model::attachment::Attachment;
use crate::domain::model::chat_session::ChatSession;
use crate::domain::model::message::Message;
use crate::domain::model::role::Role;
use crate::domain::model::token_usage::TokenUsage;
use crate::domain::port::llm_provider::LlmProvider;
use crate::domain::repository::chat_message_repository::ChatMessageRepository;
use crate::domain::repository::chat_session_repository::ChatSessionRepository;
use crate::domain::repository::token_usage_repository::TokenUsageRepository;
use crate::domain::service::agent_service::{AgentProgressEvent, AgentService};
use crate::domain::service::context_service::ContextService;
use tokio::sync::mpsc;
use uuid::Uuid;

#[derive(Debug)]
pub struct HandleAgentInput {
    pub session_id: Uuid,
    pub user_input: String,
    pub attachments: Vec<Attachment>,
}

#[derive(Debug)]
pub struct HandleAgentOutput {
    pub reply: Vec<AgentEvent>,
    pub usage: TokenUsage,
    pub context_input_tokens: u64,
    pub context_window_tokens: u64,
    pub context_percent_used: u64,
}

#[derive(Debug)]
pub enum AgentEvent {
    AssistantMessage(String),
}

pub struct AgentUsecase<L, S, M, T> {
    agent_service: AgentService<L>,
    context_service: ContextService<L>,
    chat_session_repository: S,
    chat_message_repository: M,
    token_usage_repository: T,
}

impl<L, S, M, T> AgentUsecase<L, S, M, T>
where
    L: LlmProvider,
    S: ChatSessionRepository,
    M: ChatMessageRepository,
    T: TokenUsageRepository,
{
    pub fn new(
        agent_service: AgentService<L>,
        context_service: ContextService<L>,
        chat_session_repository: S,
        chat_message_repository: M,
        token_usage_repository: T,
    ) -> Self {
        Self {
            agent_service,
            context_service,
            chat_session_repository,
            chat_message_repository,
            token_usage_repository,
        }
    }

    pub async fn start_session(&self) -> Result<ChatSession, AgentUsecaseError> {
        self.chat_session_repository
            .create()
            .await
            .map_err(Into::into)
    }

    pub async fn find_session(
        &self,
        session_id: Uuid,
    ) -> Result<Option<ChatSession>, AgentUsecaseError> {
        self.chat_session_repository
            .find_by_id(session_id)
            .await
            .map_err(Into::into)
    }

    pub async fn list_sessions(&self, limit: usize) -> Result<Vec<ChatSession>, AgentUsecaseError> {
        self.chat_session_repository
            .list_recent(limit)
            .await
            .map_err(Into::into)
    }

    pub async fn handle(
        &self,
        input: HandleAgentInput,
        tx: mpsc::Sender<AgentProgressEvent>,
    ) -> Result<HandleAgentOutput, AgentUsecaseError> {
        let history_entries = self
            .chat_message_repository
            .list_for_session(input.session_id)
            .await?;

        let history = history_entries
            .into_iter()
            .map(|entry| entry.message)
            .collect::<Vec<Message>>();

        let last_usage = self
            .token_usage_repository
            .find_latest_for_session(input.session_id)
            .await?;

        let context_messages = self
            .context_service
            .build_context(history, last_usage)
            .await?;

        let user_message = if input.attachments.is_empty() {
            Message::text(Role::User, input.user_input.clone())
        } else {
            Message::multimodal(
                Role::User,
                input.user_input.clone(),
                input.attachments.clone(),
            )
        };

        self.chat_message_repository
            .append(input.session_id, user_message.clone())
            .await?;

        let result = self
            .agent_service
            .run(context_messages, user_message, tx)
            .await?;

        let context_input_tokens = result.last_input_tokens;
        let context_window_tokens = self.context_service.context_window_tokens();
        let context_percent_used = self.context_service.percent_used(context_input_tokens);

        for turn_message in result.messages {
            let saved_message = self
                .chat_message_repository
                .append(input.session_id, turn_message.message)
                .await?;

            if let Some(usage) = turn_message.usage
                && !usage.tokens.is_empty()
            {
                self.token_usage_repository
                    .record_for_message(saved_message.id, &usage.model, usage.tokens)
                    .await?;
            }
        }

        let final_text = result.final_text.clone();
        let usage = result.usage;

        Ok(HandleAgentOutput {
            reply: vec![AgentEvent::AssistantMessage(final_text)],
            usage,
            context_input_tokens,
            context_window_tokens,
            context_percent_used,
        })
    }
}
