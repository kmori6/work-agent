use crate::application::error::agent_usecase_error::AgentUsecaseError;
use crate::domain::model::chat_session::ChatSession;
use crate::domain::model::input_file::InputFile;
use crate::domain::model::input_image::InputImage;
use crate::domain::model::message::{Message, MessageContent};
use crate::domain::model::role::Role;
use crate::domain::model::token_usage::TokenUsage;
use crate::domain::model::tool_approval::{
    ToolApproval, ToolApprovalDecision, ToolApprovalRequest,
};
use crate::domain::port::llm_provider::LlmProvider;
use crate::domain::port::tool::ToolExecutionPolicy;
use crate::domain::repository::chat_message_repository::ChatMessageRepository;
use crate::domain::repository::chat_session_repository::ChatSessionRepository;
use crate::domain::repository::token_usage_repository::TokenUsageRepository;
use crate::domain::repository::tool_approval_repository::ToolApprovalRepository;
use crate::domain::service::agent_service::{
    AgentApprovalRequired, AgentEvent as AgentProgressEvent, AgentOutput, AgentService,
};
use crate::domain::service::compaction_service::CompactionService;
use crate::domain::service::instruction_service::InstructionService;
use crate::domain::service::tool_service::ToolRuleSummary;
use std::collections::HashMap;
use tokio::sync::{Mutex, mpsc};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub enum Attachment {
    Image(InputImage),
    File(InputFile),
}

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
    instruction_service: InstructionService,
    compaction_service: CompactionService<L>,
    chat_session_repository: S,
    chat_message_repository: M,
    token_usage_repository: T,
    tool_approval_repository: A,
    pending_approvals: Mutex<HashMap<Uuid, AgentApprovalRequired>>,
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
        instruction_service: InstructionService,
        compaction_service: CompactionService<L>,
        chat_session_repository: S,
        chat_message_repository: M,
        token_usage_repository: T,
        tool_approval_repository: A,
    ) -> Self {
        Self {
            agent_service,
            instruction_service,
            compaction_service,
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

    pub fn tool_names(&self) -> Vec<String> {
        self.agent_service.tool_service().tool_names()
    }

    pub async fn handle(
        &self,
        input: HandleAgentInput,
        tx: mpsc::Sender<AgentProgressEvent>,
    ) -> Result<HandleAgentOutput, AgentUsecaseError> {
        // 1. Reject new user input while a tool approval is pending.
        {
            let pending_approvals = self.pending_approvals.lock().await;
            if pending_approvals.contains_key(&input.session_id) {
                return Err(AgentUsecaseError::ApprovalPending(input.session_id));
            }
        }

        // 2. Load conversation history and latest token usage.
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

        // 3. Build the LLM context from the stored history.
        let context_messages = self
            .compaction_service
            .compact_if_needed(history, last_usage)
            .await?;

        // 4. Build and save the new user message.
        let user_message = build_user_message(&input)?;

        self.chat_message_repository
            .append(input.session_id, user_message.clone())
            .await?;

        // 5. Run the agent with context + the new user message.
        let mut agent_messages = context_messages;
        agent_messages.push(user_message);

        let instruction = self.instruction_service.build_agent_instruction();
        let output = self
            .agent_service
            .run(instruction, agent_messages, tx)
            .await?;

        match output {
            AgentOutput::Completed(completion) => {
                // 6.1 Extract the final assistant text for the UI.
                let final_text = final_assistant_text(&completion.messages).unwrap_or_default();

                // 6.2 Save all agent-produced messages and remember the last saved message.
                let message_count = completion.messages.len();
                let mut last_message_id = None;

                for (index, message) in completion.messages.into_iter().enumerate() {
                    let saved_message = self
                        .chat_message_repository
                        .append(input.session_id, message)
                        .await?;

                    if index + 1 == message_count {
                        last_message_id = Some(saved_message.id);
                    }
                }

                // 6.3 Attach token usage to the last saved agent message.
                if let Some(message_id) = last_message_id
                    && !completion.usage.is_empty()
                {
                    self.token_usage_repository
                        .record_for_message(
                            message_id,
                            self.agent_service.model(),
                            completion.usage,
                        )
                        .await?;
                }

                // 6.4 Build and return the UI output for a completed run.
                Ok(HandleAgentOutput {
                    events: vec![AgentEvent::AssistantMessage(final_text)],
                    usage: completion.usage,
                    context_input_tokens: completion.usage.input_tokens,
                    context_window_tokens: self.compaction_service.context_window_tokens(),
                    context_percent_used: self
                        .compaction_service
                        .percent_used(completion.usage.input_tokens),
                })
            }
            AgentOutput::ApprovalRequired(required) => {
                // 7.1 Convert the tool approval request into a UI event.
                let event = AgentEvent::ToolConfirmationRequested {
                    call_id: required.request.call_id.clone(),
                    tool_name: required.request.tool_name.clone(),
                    arguments: required.request.arguments.clone(),
                    policy: required.request.policy,
                };

                // 7.2 Store the pending approval so /approve or /deny can resume the run.
                let mut pending_approvals = self.pending_approvals.lock().await;
                pending_approvals.insert(input.session_id, *required);

                // 7.3 Build and return the UI output for a paused run.
                let context_input_tokens = last_usage.map_or(0, |usage| usage.input_tokens);

                Ok(HandleAgentOutput {
                    events: vec![event],
                    usage: TokenUsage::default(),
                    context_input_tokens,
                    context_window_tokens: self.compaction_service.context_window_tokens(),
                    context_percent_used: self
                        .compaction_service
                        .percent_used(context_input_tokens),
                })
            }
        }
    }

    async fn get_pending_approval(
        &self,
        session_id: Uuid,
    ) -> Result<AgentApprovalRequired, AgentUsecaseError> {
        let pending_approvals = self.pending_approvals.lock().await;

        pending_approvals
            .get(&session_id)
            .cloned()
            .ok_or(AgentUsecaseError::ApprovalNotPending(session_id))
    }

    async fn clear_pending_approval(&self, session_id: Uuid) {
        let mut pending_approvals = self.pending_approvals.lock().await;
        pending_approvals.remove(&session_id);
    }

    pub async fn deny_approval(
        &self,
        session_id: Uuid,
        tx: mpsc::Sender<AgentProgressEvent>,
    ) -> Result<HandleAgentOutput, AgentUsecaseError> {
        self.resume_approval(session_id, ToolApprovalDecision::Denied, "/deny", tx)
            .await
    }

    pub async fn approve_approval(
        &self,
        session_id: Uuid,
        tx: mpsc::Sender<AgentProgressEvent>,
    ) -> Result<HandleAgentOutput, AgentUsecaseError> {
        self.resume_approval(session_id, ToolApprovalDecision::Approved, "/approve", tx)
            .await
    }

    async fn resume_approval(
        &self,
        session_id: Uuid,
        decision: ToolApprovalDecision,
        command_text: &'static str,
        tx: mpsc::Sender<AgentProgressEvent>,
    ) -> Result<HandleAgentOutput, AgentUsecaseError> {
        let pending = self.get_pending_approval(session_id).await?;

        self.record_tool_approval(session_id, &pending.request, decision)
            .await?;
        self.save_user_text(session_id, command_text).await?;

        let latest_usage = self
            .token_usage_repository
            .find_latest_for_session(session_id)
            .await?;
        let fallback_context_input_tokens = latest_usage.map_or(0, |usage| usage.input_tokens);

        let output = self
            .agent_service
            .resume(pending.continuation.clone(), decision, tx)
            .await?;

        match output {
            AgentOutput::Completed(completion) => {
                let final_text = final_assistant_text(&completion.messages).unwrap_or_default();
                let message_count = completion.messages.len();
                let mut last_message_id = None;

                for (index, message) in completion.messages.into_iter().enumerate() {
                    let saved_message = self
                        .chat_message_repository
                        .append(session_id, message)
                        .await?;

                    if index + 1 == message_count {
                        last_message_id = Some(saved_message.id);
                    }
                }

                if let Some(message_id) = last_message_id
                    && !completion.usage.is_empty()
                {
                    self.token_usage_repository
                        .record_for_message(
                            message_id,
                            self.agent_service.model(),
                            completion.usage,
                        )
                        .await?;
                }

                self.clear_pending_approval(session_id).await;

                Ok(HandleAgentOutput {
                    events: vec![AgentEvent::AssistantMessage(final_text)],
                    usage: completion.usage,
                    context_input_tokens: completion.usage.input_tokens,
                    context_window_tokens: self.compaction_service.context_window_tokens(),
                    context_percent_used: self
                        .compaction_service
                        .percent_used(completion.usage.input_tokens),
                })
            }
            AgentOutput::ApprovalRequired(required) => {
                let event = AgentEvent::ToolConfirmationRequested {
                    call_id: required.request.call_id.clone(),
                    tool_name: required.request.tool_name.clone(),
                    arguments: required.request.arguments.clone(),
                    policy: required.request.policy,
                };

                let mut pending_approvals = self.pending_approvals.lock().await;
                pending_approvals.insert(session_id, *required);

                Ok(HandleAgentOutput {
                    events: vec![event],
                    usage: TokenUsage::default(),
                    context_input_tokens: fallback_context_input_tokens,
                    context_window_tokens: self.compaction_service.context_window_tokens(),
                    context_percent_used: self
                        .compaction_service
                        .percent_used(fallback_context_input_tokens),
                })
            }
        }
    }

    async fn save_user_text(
        &self,
        session_id: Uuid,
        text: impl Into<String>,
    ) -> Result<(), AgentUsecaseError> {
        let message = Message::input_text(text.into())?;

        self.chat_message_repository
            .append(session_id, message)
            .await?;

        Ok(())
    }
    async fn record_tool_approval(
        &self,
        session_id: Uuid,
        request: &ToolApprovalRequest,
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

    pub async fn tool_rule_summaries(&self) -> Result<Vec<ToolRuleSummary>, AgentUsecaseError> {
        self.agent_service
            .tool_service()
            .tool_rule_summaries()
            .await
            .map_err(Into::into)
    }
}

fn build_user_message(input: &HandleAgentInput) -> Result<Message, AgentUsecaseError> {
    let mut contents = Vec::with_capacity(input.attachments.len() + 1);

    contents.push(MessageContent::InputText(input.user_input.clone()));
    contents.extend(input.attachments.iter().map(attachment_to_message_content));

    Ok(Message::new(Role::User, contents)?)
}

fn attachment_to_message_content(attachment: &Attachment) -> MessageContent {
    match attachment {
        Attachment::Image(image) => MessageContent::InputImage(image.clone()),
        Attachment::File(file) => MessageContent::InputFile(file.clone()),
    }
}

fn final_assistant_text(messages: &[Message]) -> Option<String> {
    messages
        .iter()
        .rev()
        .filter(|message| message.role == Role::Assistant)
        .flat_map(|message| message.contents.iter().rev())
        .find_map(|content| match content {
            MessageContent::OutputText(text) => Some(text.clone()),
            _ => None,
        })
}
