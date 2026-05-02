use crate::application::error::agent_usecase_error::AgentUsecaseError;
use crate::domain::error::agent_error::AgentError;
use crate::domain::model::awaiting_tool_approval::AwaitingToolApproval;
use crate::domain::model::chat_message::ChatMessage;
use crate::domain::model::chat_session::{ChatSession, ChatSessionStatus};
use crate::domain::model::chat_session_event::ChatSessionEvent;
use crate::domain::model::input_file::InputFile;
use crate::domain::model::input_image::InputImage;
use crate::domain::model::message::{Message, MessageContent};
use crate::domain::model::role::Role;
use crate::domain::model::token_usage::TokenUsage;
use crate::domain::model::tool_approval::{ToolApproval, ToolApprovalDecision};
use crate::domain::model::tool_call::{ToolCall, ToolCallOutput};
use crate::domain::model::tool_execution_decision::ToolExecutionDecision;
use crate::domain::port::llm_provider::{LlmProvider, LlmResponse};
use crate::domain::repository::awaiting_tool_approval_repository::AwaitingToolApprovalRepository;
use crate::domain::repository::chat_message_repository::ChatMessageRepository;
use crate::domain::repository::chat_session_repository::ChatSessionRepository;
use crate::domain::repository::token_usage_repository::TokenUsageRepository;
use crate::domain::repository::tool_approval_repository::ToolApprovalRepository;
use crate::domain::service::agent_service::AgentService;
use crate::domain::service::compaction_service::CompactionService;
use crate::domain::service::instruction_service::InstructionService;
use crate::domain::service::tool_service::ToolRuleSummary;
use serde_json::json;
use tokio::sync::mpsc;
use uuid::Uuid;

// Current agent turn flow.
//
// start_turn:
//   - Validate the user message.
//   - Reject the turn unless the session is Idle.
//   - Mark the session Running.
//   - Save the user message.
//   - Build the initial compacted LLM context from the DB transcript.
//   - Delegate the rest of the turn to agent_loop.
//
// agent_loop:
//   - Call the LLM for one step.
//   - Save the assistant message and token usage before handling tools.
//   - Emit assistant text events for UI/API consumers.
//   - For allowed tool calls, execute the tool, save ToolCallOutput, then loop.
//   - For denied or errored tool calls, save an error ToolCallOutput, then loop.
//   - For tool calls that require approval, save awaiting_tool_approvals,
//     mark the session AwaitingApproval, emit a confirmation event, and stop.
//   - If the assistant message has no tool calls, mark the session Idle and finish.
//
// continue_after_tool_output:
//   - Before calling the LLM again, scan the DB transcript for already-saved
//     unresolved tool calls.
//   - If one exists, process it first. This preserves the order of multiple
//     tool calls from the same assistant message after an approval resumes.
//   - If none exists, rebuild context from the DB transcript and enter agent_loop.
//
// approve_approval:
//   - Load the awaiting approval from the DB.
//   - Restore the original ToolCall from the saved assistant message.
//   - Mark the session Running.
//   - Execute the approved tool and save ToolCallOutput.
//   - Record the approval audit row and delete awaiting_tool_approvals.
//   - Continue after the saved tool output, processing any remaining unresolved
//     tool calls before the next LLM step.
//
// deny_approval:
//   - Load the awaiting approval from the DB.
//   - Restore the original ToolCall from the saved assistant message.
//   - Mark the session Running.
//   - Save a denied ToolCallOutput without executing the tool.
//   - Record the denial audit row and delete awaiting_tool_approvals.
//   - Continue after the saved tool output, processing any remaining unresolved
//     tool calls before the next LLM step.

const MAX_LLM_STEPS: usize = 20;

#[derive(Debug, Clone)]
pub enum Attachment {
    Image(InputImage),
    File(InputFile),
}

struct AwaitingToolCall {
    tool_call: ToolCall,
}

struct UnresolvedToolCall {
    assistant_message_id: Uuid,
    tool_call: ToolCall,
}

enum ToolCallStep {
    Continued,
    AwaitingApproval(AgentStartTurnOutput),
}

#[derive(Debug)]
pub struct HandleAgentInput {
    pub session_id: Uuid,
    pub user_input: String,
    pub attachments: Vec<Attachment>,
}

#[derive(Debug)]
pub struct HandleAgentOutput {
    pub events: Vec<ChatSessionEvent>,
    pub usage: TokenUsage,
    pub context_input_tokens: u64,
    pub context_window_tokens: u64,
    pub context_percent_used: u64,
}

#[derive(Debug)]
pub struct AgentStartTurnOutput {
    pub events: Vec<ChatSessionEvent>,
}

pub struct AgentUsecase<L, S, M, T, A, W> {
    agent_service: AgentService<L>,
    instruction_service: InstructionService,
    compaction_service: CompactionService<L>,
    chat_session_repository: S,
    chat_message_repository: M,
    token_usage_repository: T,
    tool_approval_repository: A,
    awaiting_tool_approval_repository: W,
}

impl<L, S, M, T, A, W> AgentUsecase<L, S, M, T, A, W>
where
    L: LlmProvider,
    S: ChatSessionRepository,
    M: ChatMessageRepository,
    T: TokenUsageRepository,
    A: ToolApprovalRepository,
    W: AwaitingToolApprovalRepository,
{
    pub fn new(
        agent_service: AgentService<L>,
        instruction_service: InstructionService,
        compaction_service: CompactionService<L>,
        chat_session_repository: S,
        chat_message_repository: M,
        token_usage_repository: T,
        tool_approval_repository: A,
        awaiting_tool_approval_repository: W,
    ) -> Self {
        Self {
            agent_service,
            instruction_service,
            compaction_service,
            chat_session_repository,
            chat_message_repository,
            token_usage_repository,
            tool_approval_repository,
            awaiting_tool_approval_repository,
        }
    }

    pub async fn submit_user_message(
        &self,
        session_id: Uuid,
        user_message: Message,
    ) -> Result<ChatMessage, AgentUsecaseError> {
        validate_user_message(&user_message)?;

        self.validate_startable_session(session_id).await?;

        self.chat_session_repository
            .update_status(session_id, ChatSessionStatus::Running)
            .await?;

        let saved_user_message = self
            .chat_message_repository
            .append(session_id, user_message)
            .await?;

        Ok(saved_user_message)
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

    pub async fn start_turn(
        &self,
        session_id: Uuid,
        user_message: ChatMessage,
        tx: mpsc::Sender<ChatSessionEvent>,
    ) -> Result<AgentStartTurnOutput, AgentUsecaseError> {
        let input_messages = self
            .load_compacted_input_messages(session_id, &user_message)
            .await?;

        let instruction = self.instruction_service.build_agent_instruction();

        self.agent_loop(session_id, instruction, input_messages, tx)
            .await
    }

    pub async fn handle(
        &self,
        input: HandleAgentInput,
        tx: mpsc::Sender<ChatSessionEvent>,
    ) -> Result<HandleAgentOutput, AgentUsecaseError> {
        let session_id = input.session_id;
        let user_message = build_user_message(&input)?;
        let saved_user_message = self.submit_user_message(session_id, user_message).await?;
        let output = self.start_turn(session_id, saved_user_message, tx).await?;

        let usage = self
            .token_usage_repository
            .find_latest_for_session(session_id)
            .await?
            .unwrap_or_default();

        Ok(HandleAgentOutput {
            events: output.events,
            context_input_tokens: usage.input_tokens,
            context_window_tokens: self.compaction_service.context_window_tokens(),
            context_percent_used: self.compaction_service.percent_used(usage.input_tokens),
            usage,
        })
    }

    pub async fn approve_approval(
        &self,
        session_id: Uuid,
        tx: mpsc::Sender<ChatSessionEvent>,
    ) -> Result<AgentStartTurnOutput, AgentUsecaseError> {
        self.resolve_awaiting_approval(session_id, ToolApprovalDecision::Approved, tx)
            .await
    }

    pub async fn deny_approval(
        &self,
        session_id: Uuid,
        tx: mpsc::Sender<ChatSessionEvent>,
    ) -> Result<AgentStartTurnOutput, AgentUsecaseError> {
        self.resolve_awaiting_approval(session_id, ToolApprovalDecision::Denied, tx)
            .await
    }

    pub async fn tool_rule_summaries(&self) -> Result<Vec<ToolRuleSummary>, AgentUsecaseError> {
        self.agent_service
            .tool_service()
            .tool_rule_summaries()
            .await
            .map_err(Into::into)
    }

    async fn validate_startable_session(&self, session_id: Uuid) -> Result<(), AgentUsecaseError> {
        let session = self
            .chat_session_repository
            .find_by_id(session_id)
            .await?
            .ok_or(AgentUsecaseError::SessionNotFound(session_id))?;

        match session.status {
            ChatSessionStatus::Idle => Ok(()),
            ChatSessionStatus::Running => Err(AgentUsecaseError::SessionStatus(
                "session is already running".to_string(),
            )),
            ChatSessionStatus::AwaitingApproval => Err(AgentUsecaseError::SessionStatus(
                "tool approval is pending".to_string(),
            )),
        }
    }

    async fn load_compacted_input_messages(
        &self,
        session_id: Uuid,
        saved_user_message: &ChatMessage,
    ) -> Result<Vec<Message>, AgentUsecaseError> {
        let history_entries = self
            .chat_message_repository
            .list_for_session(session_id)
            .await?;

        let history = history_entries
            .into_iter()
            .map(|entry| {
                if entry.id == saved_user_message.id {
                    saved_user_message.message.clone()
                } else {
                    entry.message
                }
            })
            .collect::<Vec<_>>();

        let latest_usage = self
            .token_usage_repository
            .find_latest_for_session(session_id)
            .await?;

        self.compaction_service
            .compact_if_needed(history, latest_usage)
            .await
            .map_err(Into::into)
    }

    async fn save_llm_response(
        &self,
        session_id: Uuid,
        response: &LlmResponse,
    ) -> Result<ChatMessage, AgentUsecaseError> {
        let saved_message = self
            .chat_message_repository
            .append(session_id, response.message.clone())
            .await?;

        if !response.usage.is_empty() {
            self.token_usage_repository
                .record_for_message(saved_message.id, self.agent_service.model(), response.usage)
                .await?;
        }

        Ok(saved_message)
    }

    async fn save_tool_call_output(
        &self,
        session_id: Uuid,
        output: ToolCallOutput,
    ) -> Result<ChatMessage, AgentUsecaseError> {
        let message = Message::tool_call_outputs(vec![output])?;

        self.chat_message_repository
            .append(session_id, message)
            .await
            .map_err(Into::into)
    }

    async fn execute_and_save_tool_call(
        &self,
        session_id: Uuid,
        tool_call: ToolCall,
        tx: &mpsc::Sender<ChatSessionEvent>,
    ) -> Result<(), AgentUsecaseError> {
        let call_id = tool_call.call_id.clone();
        let tool_name = tool_call.name.clone();

        let _ = tx
            .send(ChatSessionEvent::ToolCallStarted {
                session_id,
                call_id: call_id.clone(),
                tool_name: tool_name.clone(),
            })
            .await;

        let result = self.agent_service.tool_service().execute(tool_call).await;

        let tool_call_output = match result {
            Ok(output) => output,
            Err(err) => tool_call_error_output(call_id.clone(), err.to_string()),
        };

        let output = tool_call_output.output.clone();
        let status = tool_call_output.status;

        self.save_tool_call_output(session_id, tool_call_output)
            .await?;

        let _ = tx
            .send(ChatSessionEvent::ToolCallFinished {
                session_id,
                call_id,
                tool_name,
                output,
                status,
            })
            .await;

        Ok(())
    }

    async fn load_compacted_session_messages(
        &self,
        session_id: Uuid,
    ) -> Result<Vec<Message>, AgentUsecaseError> {
        let history_entries = self
            .chat_message_repository
            .list_for_session(session_id)
            .await?;

        let history = history_entries
            .into_iter()
            .map(|entry| entry.message)
            .collect::<Vec<_>>();

        let latest_usage = self
            .token_usage_repository
            .find_latest_for_session(session_id)
            .await?;

        self.compaction_service
            .compact_if_needed(history, latest_usage)
            .await
            .map_err(Into::into)
    }

    async fn agent_loop(
        &self,
        session_id: Uuid,
        instruction: String,
        mut input_messages: Vec<Message>,
        tx: mpsc::Sender<ChatSessionEvent>,
    ) -> Result<AgentStartTurnOutput, AgentUsecaseError> {
        let mut events = Vec::new();

        for _ in 0..MAX_LLM_STEPS {
            let _ = tx.send(ChatSessionEvent::LlmStarted { session_id }).await;

            let llm_response = self
                .agent_service
                .llm_step(instruction.clone(), input_messages.clone())
                .await?;

            let _ = tx.send(ChatSessionEvent::LlmFinished { session_id }).await;

            let saved_agent_message = self.save_llm_response(session_id, &llm_response).await?;

            for event in
                assistant_text_events(session_id, saved_agent_message.id, &llm_response.message)
            {
                let _ = tx.send(event.clone()).await;
                events.push(event);
            }

            let tool_calls = tool_calls_from_message(&llm_response.message);

            if tool_calls.is_empty() {
                self.chat_session_repository
                    .update_status(session_id, ChatSessionStatus::Idle)
                    .await?;

                let event = ChatSessionEvent::AgentTurnCompleted { session_id };
                let _ = tx.send(event.clone()).await;
                events.push(event);

                return Ok(AgentStartTurnOutput { events });
            }

            let mut saved_tool_output = false;

            for tool_call in tool_calls {
                match self
                    .process_tool_call(
                        session_id,
                        saved_agent_message.id,
                        tool_call,
                        &mut events,
                        &tx,
                    )
                    .await?
                {
                    ToolCallStep::Continued => {
                        saved_tool_output = true;
                    }
                    ToolCallStep::AwaitingApproval(output) => {
                        return Ok(output);
                    }
                }
            }

            if saved_tool_output {
                input_messages = self.load_compacted_session_messages(session_id).await?;
                continue;
            }
        }

        self.chat_session_repository
            .update_status(session_id, ChatSessionStatus::Idle)
            .await?;

        Err(AgentUsecaseError::Agent(AgentError::MaxToolIterations(
            MAX_LLM_STEPS,
        )))
    }

    async fn load_awaiting_tool_call(
        &self,
        session_id: Uuid,
    ) -> Result<AwaitingToolCall, AgentUsecaseError> {
        let awaiting = self
            .awaiting_tool_approval_repository
            .find_by_session_id(session_id)
            .await?
            .ok_or(AgentUsecaseError::ApprovalNotPending(session_id))?;

        let messages = self
            .chat_message_repository
            .list_for_session(session_id)
            .await?;

        let assistant_message = messages
            .into_iter()
            .find(|entry| entry.id == awaiting.assistant_message_id)
            .ok_or_else(|| {
                AgentUsecaseError::SessionStatus(format!(
                    "awaiting approval assistant message not found: {}",
                    awaiting.assistant_message_id
                ))
            })?;

        let tool_call = tool_call_from_message(&assistant_message.message, &awaiting.tool_call_id)
            .ok_or_else(|| {
                AgentUsecaseError::SessionStatus(format!(
                    "awaiting approval tool call not found: {}",
                    awaiting.tool_call_id
                ))
            })?;

        Ok(AwaitingToolCall { tool_call })
    }

    async fn validate_awaiting_approval_session(
        &self,
        session_id: Uuid,
    ) -> Result<(), AgentUsecaseError> {
        let session = self
            .chat_session_repository
            .find_by_id(session_id)
            .await?
            .ok_or(AgentUsecaseError::SessionNotFound(session_id))?;

        match session.status {
            ChatSessionStatus::AwaitingApproval => Ok(()),
            ChatSessionStatus::Idle => Err(AgentUsecaseError::ApprovalNotPending(session_id)),
            ChatSessionStatus::Running => Err(AgentUsecaseError::SessionStatus(
                "session is already running".to_string(),
            )),
        }
    }

    async fn save_denied_tool_call_output(
        &self,
        session_id: Uuid,
        tool_call: &ToolCall,
    ) -> Result<ToolCallOutput, AgentUsecaseError> {
        let output = tool_call_error_output(
            tool_call.call_id.clone(),
            "tool execution was denied by user",
        );

        self.save_tool_call_output(session_id, output.clone())
            .await?;

        Ok(output)
    }

    async fn record_tool_approval_from_tool_call(
        &self,
        session_id: Uuid,
        tool_call: &ToolCall,
        decision: ToolApprovalDecision,
    ) -> Result<(), AgentUsecaseError> {
        self.tool_approval_repository
            .record(ToolApproval {
                session_id,
                tool_call_id: tool_call.call_id.clone(),
                tool_name: tool_call.name.clone(),
                arguments: tool_call.arguments.clone(),
                decision,
            })
            .await?;

        Ok(())
    }

    async fn resolve_awaiting_approval(
        &self,
        session_id: Uuid,
        decision: ToolApprovalDecision,
        tx: mpsc::Sender<ChatSessionEvent>,
    ) -> Result<AgentStartTurnOutput, AgentUsecaseError> {
        self.validate_awaiting_approval_session(session_id).await?;

        let awaiting = self.load_awaiting_tool_call(session_id).await?;
        let tool_call = awaiting.tool_call;

        self.chat_session_repository
            .update_status(session_id, ChatSessionStatus::Running)
            .await?;

        let resolved = ChatSessionEvent::ToolCallApprovalResolved {
            session_id,
            call_id: tool_call.call_id.clone(),
            tool_name: tool_call.name.clone(),
            decision,
        };
        let _ = tx.send(resolved).await;

        match decision {
            ToolApprovalDecision::Approved => {
                self.execute_and_save_tool_call(session_id, tool_call.clone(), &tx)
                    .await?;
            }
            ToolApprovalDecision::Denied => {
                let output = self
                    .save_denied_tool_call_output(session_id, &tool_call)
                    .await?;

                let _ = tx
                    .send(ChatSessionEvent::ToolCallFinished {
                        session_id,
                        call_id: tool_call.call_id.clone(),
                        tool_name: tool_call.name.clone(),
                        output: output.output,
                        status: output.status,
                    })
                    .await;
            }
        }

        self.record_tool_approval_from_tool_call(session_id, &tool_call, decision)
            .await?;

        self.awaiting_tool_approval_repository
            .delete_by_session_id(session_id)
            .await?;

        self.continue_after_tool_output(session_id, tx).await
    }

    async fn next_unresolved_tool_call(
        &self,
        session_id: Uuid,
    ) -> Result<Option<UnresolvedToolCall>, AgentUsecaseError> {
        let messages = self
            .chat_message_repository
            .list_for_session(session_id)
            .await?;

        let mut resolved_call_ids = std::collections::HashSet::new();

        for entry in &messages {
            for content in &entry.message.content {
                if let MessageContent::ToolCallOutput(output) = content {
                    resolved_call_ids.insert(output.call_id.clone());
                }
            }
        }

        for entry in messages {
            if entry.message.role != Role::Assistant {
                continue;
            }

            for content in entry.message.content {
                if let MessageContent::ToolCall(call) = content
                    && !resolved_call_ids.contains(&call.call_id)
                {
                    return Ok(Some(UnresolvedToolCall {
                        assistant_message_id: entry.id,
                        tool_call: call,
                    }));
                }
            }
        }

        Ok(None)
    }

    async fn process_tool_call(
        &self,
        session_id: Uuid,
        assistant_message_id: Uuid,
        tool_call: ToolCall,
        events: &mut Vec<ChatSessionEvent>,
        tx: &mpsc::Sender<ChatSessionEvent>,
    ) -> Result<ToolCallStep, AgentUsecaseError> {
        match self
            .agent_service
            .tool_service()
            .decide_execution(&tool_call)
            .await
        {
            Ok(ToolExecutionDecision::Allow) => {
                self.execute_and_save_tool_call(session_id, tool_call, tx)
                    .await?;

                Ok(ToolCallStep::Continued)
            }
            Ok(ToolExecutionDecision::Ask) => {
                let policy = self
                    .agent_service
                    .tool_service()
                    .check_execution_policy(&tool_call)?;

                self.awaiting_tool_approval_repository
                    .save(AwaitingToolApproval {
                        session_id,
                        assistant_message_id,
                        tool_call_id: tool_call.call_id.clone(),
                    })
                    .await?;

                let event = ChatSessionEvent::ToolCallApprovalRequested {
                    session_id,
                    call_id: tool_call.call_id,
                    tool_name: tool_call.name,
                    arguments: tool_call.arguments,
                    policy,
                };

                let _ = tx.send(event.clone()).await;
                events.push(event);

                self.chat_session_repository
                    .update_status(session_id, ChatSessionStatus::AwaitingApproval)
                    .await?;

                Ok(ToolCallStep::AwaitingApproval(AgentStartTurnOutput {
                    events: std::mem::take(events),
                }))
            }
            Ok(ToolExecutionDecision::Deny) => {
                let output = tool_call_error_output(
                    tool_call.call_id.clone(),
                    "tool execution was blocked by execution rule",
                );

                self.save_tool_call_output(session_id, output.clone())
                    .await?;

                let _ = tx
                    .send(ChatSessionEvent::ToolCallFinished {
                        session_id,
                        call_id: tool_call.call_id,
                        tool_name: tool_call.name,
                        output: output.output,
                        status: output.status,
                    })
                    .await;

                Ok(ToolCallStep::Continued)
            }
            Err(err) => {
                let output = tool_call_error_output(tool_call.call_id.clone(), err.to_string());

                self.save_tool_call_output(session_id, output.clone())
                    .await?;

                let _ = tx
                    .send(ChatSessionEvent::ToolCallFinished {
                        session_id,
                        call_id: tool_call.call_id,
                        tool_name: tool_call.name,
                        output: output.output,
                        status: output.status,
                    })
                    .await;

                Ok(ToolCallStep::Continued)
            }
        }
    }

    async fn continue_after_tool_output(
        &self,
        session_id: Uuid,
        tx: mpsc::Sender<ChatSessionEvent>,
    ) -> Result<AgentStartTurnOutput, AgentUsecaseError> {
        let mut events = Vec::new();

        loop {
            if let Some(unresolved) = self.next_unresolved_tool_call(session_id).await? {
                match self
                    .process_tool_call(
                        session_id,
                        unresolved.assistant_message_id,
                        unresolved.tool_call,
                        &mut events,
                        &tx,
                    )
                    .await?
                {
                    ToolCallStep::Continued => continue,
                    ToolCallStep::AwaitingApproval(output) => return Ok(output),
                }
            }

            let input_messages = self.load_compacted_session_messages(session_id).await?;
            let instruction = self.instruction_service.build_agent_instruction();

            let output = self
                .agent_loop(session_id, instruction, input_messages, tx)
                .await?;

            let mut all_events = events;
            all_events.extend(output.events);

            return Ok(AgentStartTurnOutput { events: all_events });
        }
    }

    pub async fn list_awaiting_approvals(
        &self,
    ) -> Result<Vec<AwaitingToolApproval>, AgentUsecaseError> {
        self.awaiting_tool_approval_repository
            .list_all()
            .await
            .map_err(Into::into)
    }
}

fn build_user_message(input: &HandleAgentInput) -> Result<Message, AgentUsecaseError> {
    let mut contents = Vec::with_capacity(input.attachments.len() + 1);

    contents.push(MessageContent::InputText {
        text: input.user_input.clone(),
    });
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
        .flat_map(|message| message.content.iter().rev())
        .find_map(|content| match content {
            MessageContent::OutputText { text } => Some(text.clone()),
            _ => None,
        })
}

fn validate_user_message(user_message: &Message) -> Result<(), AgentUsecaseError> {
    if user_message.role != Role::User {
        return Err(AgentUsecaseError::InvalidUserMessage(
            "message role must be user".to_string(),
        ));
    }

    let has_input_text = user_message
        .content
        .iter()
        .any(|content| matches!(content, MessageContent::InputText { .. }));

    if !has_input_text {
        return Err(AgentUsecaseError::InvalidUserMessage(
            "user message must contain input_text".to_string(),
        ));
    }

    let contains_only_user_input = user_message.content.iter().all(|content| {
        matches!(
            content,
            MessageContent::InputText { .. }
                | MessageContent::InputImage(_)
                | MessageContent::InputFile(_)
        )
    });

    if !contains_only_user_input {
        return Err(AgentUsecaseError::InvalidUserMessage(
            "user message can only contain input_text, input_image, or input_file".to_string(),
        ));
    }

    Ok(())
}

fn assistant_text_events(
    session_id: Uuid,
    message_id: Uuid,
    message: &Message,
) -> Vec<ChatSessionEvent> {
    message
        .content
        .iter()
        .filter_map(|content| match content {
            MessageContent::OutputText { text } if !text.is_empty() => {
                Some(ChatSessionEvent::AssistantMessageCreated {
                    session_id,
                    message_id,
                    content: text.clone(),
                })
            }
            _ => None,
        })
        .collect()
}

fn tool_calls_from_message(message: &Message) -> Vec<ToolCall> {
    message
        .content
        .iter()
        .filter_map(|content| match content {
            MessageContent::ToolCall(call) => Some(call.clone()),
            _ => None,
        })
        .collect()
}

fn tool_call_error_output(
    call_id: impl Into<String>,
    message: impl Into<String>,
) -> ToolCallOutput {
    ToolCallOutput::error(
        call_id,
        json!({
            "message": message.into(),
        }),
    )
}

fn tool_call_from_message(message: &Message, call_id: &str) -> Option<ToolCall> {
    message.content.iter().find_map(|content| match content {
        MessageContent::ToolCall(call) if call.call_id == call_id => Some(call.clone()),
        _ => None,
    })
}
