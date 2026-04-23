use crate::application::error::agent_usecase_error::AgentUsecaseError;
use crate::domain::model::attachment::Attachment;
use crate::domain::model::chat_session::ChatSession;
use crate::domain::model::message::Message;
use crate::domain::model::role::Role;
use crate::domain::model::token_usage::TokenUsage;
use crate::domain::model::tool_approval::{ToolApproval, ToolApprovalDecision};
use crate::domain::port::llm_provider::LlmProvider;
use crate::domain::port::tool::ToolExecutionPolicy;
use crate::domain::repository::chat_message_repository::ChatMessageRepository;
use crate::domain::repository::chat_session_repository::ChatSessionRepository;
use crate::domain::repository::token_usage_repository::TokenUsageRepository;
use crate::domain::repository::tool_approval_repository::ToolApprovalRepository;
use crate::domain::service::agent_service::{
    AgentApprovalRequest, AgentOutput, AgentProgressEvent, AgentService,
};
use crate::domain::service::context_service::ContextService;
use std::collections::HashMap;
use tokio::sync::{Mutex, mpsc};
use uuid::Uuid;

#[derive(Debug)]
pub struct HandleAgentInput {
    pub session_id: Uuid,
    pub user_input: String,
    pub attachments: Vec<Attachment>,
}

#[derive(Debug)]
pub struct HandleAgentOutput {
    pub events: Vec<AgentEvent>,
    pub usage: TokenUsage,
    pub context_input_tokens: u64,
    pub context_window_tokens: u64,
    pub context_percent_used: u64,
}

#[derive(Debug)]
pub enum AgentEvent {
    AssistantMessage(String),
    ToolConfirmationRequested {
        call_id: String,
        tool_name: String,
        arguments: serde_json::Value,
        policy: ToolExecutionPolicy,
    },
}

pub struct AgentUsecase<L, S, M, T, A> {
    agent_service: AgentService<L>,
    context_service: ContextService<L>,
    chat_session_repository: S,
    chat_message_repository: M,
    token_usage_repository: T,
    tool_approval_repository: A,
    pending_approvals: Mutex<HashMap<Uuid, AgentApprovalRequest>>,
}

impl<L, S, M, T, A> AgentUsecase<L, S, M, T, A>
where
    L: LlmProvider,
    S: ChatSessionRepository,
    M: ChatMessageRepository,
    T: TokenUsageRepository,
    A: ToolApprovalRepository,
{
    pub fn new(
        agent_service: AgentService<L>,
        context_service: ContextService<L>,
        chat_session_repository: S,
        chat_message_repository: M,
        token_usage_repository: T,
        tool_approval_repository: A,
    ) -> Self {
        Self {
            agent_service,
            context_service,
            chat_session_repository,
            chat_message_repository,
            token_usage_repository,
            tool_approval_repository,
            pending_approvals: Mutex::new(HashMap::new()),
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
        let has_pending_approval = {
            let pending_approvals = self.pending_approvals.lock().await;
            pending_approvals.contains_key(&input.session_id)
        };

        if has_pending_approval {
            return Err(AgentUsecaseError::ApprovalPending(input.session_id));
        }

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

        let output = self
            .agent_service
            .run(context_messages, user_message, tx)
            .await?;

        self.handle_agent_output(input.session_id, output).await
    }

    async fn handle_agent_output(
        &self,
        session_id: Uuid,
        output: AgentOutput,
    ) -> Result<HandleAgentOutput, AgentUsecaseError> {
        match output {
            AgentOutput::Completed(result) => {
                let context_input_tokens = result.last_input_tokens;
                let context_window_tokens = self.context_service.context_window_tokens();
                let context_percent_used = self.context_service.percent_used(context_input_tokens);

                for turn_message in result.messages {
                    let saved_message = self
                        .chat_message_repository
                        .append(session_id, turn_message.message)
                        .await?;

                    if let Some(usage) = turn_message.usage
                        && !usage.tokens.is_empty()
                    {
                        self.token_usage_repository
                            .record_for_message(saved_message.id, &usage.model, usage.tokens)
                            .await?;
                    }
                }

                Ok(HandleAgentOutput {
                    events: vec![AgentEvent::AssistantMessage(result.final_text)],
                    usage: result.usage,
                    context_input_tokens,
                    context_window_tokens,
                    context_percent_used,
                })
            }
            AgentOutput::ApprovalRequested(request) => {
                let context_input_tokens = request.last_input_tokens;
                let context_window_tokens = self.context_service.context_window_tokens();
                let context_percent_used = self.context_service.percent_used(context_input_tokens);

                let event = AgentEvent::ToolConfirmationRequested {
                    call_id: request.call_id.clone(),
                    tool_name: request.tool_name.clone(),
                    arguments: request.arguments.clone(),
                    policy: request.policy,
                };

                {
                    let mut pending_approvals = self.pending_approvals.lock().await;
                    pending_approvals.insert(session_id, request);
                }

                Ok(HandleAgentOutput {
                    events: vec![event],
                    usage: TokenUsage::default(),
                    context_input_tokens,
                    context_window_tokens,
                    context_percent_used,
                })
            }
        }
    }

    async fn take_pending_approval(
        &self,
        session_id: Uuid,
    ) -> Result<AgentApprovalRequest, AgentUsecaseError> {
        let mut pending_approvals = self.pending_approvals.lock().await;

        pending_approvals
            .remove(&session_id)
            .ok_or(AgentUsecaseError::ApprovalNotPending(session_id))
    }

    pub async fn deny_approval(
        &self,
        session_id: Uuid,
    ) -> Result<HandleAgentOutput, AgentUsecaseError> {
        let request = self.take_pending_approval(session_id).await?;

        self.record_tool_approval(session_id, &request, ToolApprovalDecision::Denied)
            .await?;

        self.save_user_text(session_id, "/deny").await?;

        let message = format!(
            "Tool execution was denied: {} ({})",
            request.tool_name, request.call_id
        );

        self.save_assistant_text(session_id, message.clone())
            .await?;

        Ok(HandleAgentOutput {
            events: vec![AgentEvent::AssistantMessage(message)],
            usage: TokenUsage::default(),
            context_input_tokens: request.last_input_tokens,
            context_window_tokens: self.context_service.context_window_tokens(),
            context_percent_used: self.context_service.percent_used(request.last_input_tokens),
        })
    }

    pub async fn approve_approval(
        &self,
        session_id: Uuid,
        tx: mpsc::Sender<AgentProgressEvent>,
    ) -> Result<HandleAgentOutput, AgentUsecaseError> {
        let request = self.take_pending_approval(session_id).await?;

        self.record_tool_approval(session_id, &request, ToolApprovalDecision::Approved)
            .await?;

        self.save_user_text(session_id, "/approve").await?;

        let output = self
            .agent_service
            .resume_after_approval(request, tx)
            .await?;

        self.handle_agent_output(session_id, output).await
    }

    async fn save_user_text(
        &self,
        session_id: Uuid,
        text: impl Into<String>,
    ) -> Result<(), AgentUsecaseError> {
        self.chat_message_repository
            .append(session_id, Message::text(Role::User, text.into()))
            .await?;

        Ok(())
    }

    async fn save_assistant_text(
        &self,
        session_id: Uuid,
        text: impl Into<String>,
    ) -> Result<(), AgentUsecaseError> {
        self.chat_message_repository
            .append(session_id, Message::text(Role::Assistant, text.into()))
            .await?;

        Ok(())
    }

    async fn record_tool_approval(
        &self,
        session_id: Uuid,
        request: &AgentApprovalRequest,
        decision: ToolApprovalDecision,
    ) -> Result<(), AgentUsecaseError> {
        self.tool_approval_repository
            .record(ToolApproval {
                session_id,
                tool_call_id: request.call_id.clone(),
                tool_name: request.tool_name.clone(),
                arguments: request.arguments.clone(),
                decision,
            })
            .await?;

        Ok(())
    }
}
