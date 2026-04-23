use crate::domain::error::agent_error::AgentError;
use crate::domain::model::message::Message;
use crate::domain::model::role::Role;
use crate::domain::model::token_usage::TokenUsage;
use crate::domain::model::tool::{ToolCall, ToolExecutionResult, ToolResultMessage};
use crate::domain::port::llm_provider::LlmProvider;
use crate::domain::port::tool::ToolExecutionPolicy;
use crate::domain::service::tool_service::ToolExecutor;
use futures::future::join_all;
use serde_json::Value;
use serde_json::json;
use tokio::sync::mpsc;

const DEFAULT_MODEL: &str = "global.anthropic.claude-sonnet-4-6";
const DEFAULT_MAX_TOOL_ITERATIONS: usize = 20;
const DEFAULT_SYSTEM_PROMPT: &str = "\
You are a helpful assistant.
Answer clearly and directly in Japanese.
Use available tools when they improve accuracy, especially for recent, external, or uncertain information.
After gathering what you need, respond concisely and naturally.
";

#[derive(Debug, Clone)]
pub struct AgentLlmUsage {
    pub model: String,
    pub tokens: TokenUsage,
}

#[derive(Debug, Clone)]
pub struct AgentTurnMessage {
    pub message: Message,
    pub usage: Option<AgentLlmUsage>,
}

#[derive(Debug, Clone)]
pub struct AgentResult {
    pub final_text: String,
    pub messages: Vec<AgentTurnMessage>,
    pub usage: TokenUsage,
    pub last_input_tokens: u64,
}

#[derive(Debug, Clone)]
pub enum AgentOutput {
    Completed(AgentResult),
    ApprovalRequested(AgentApprovalRequest),
}

#[derive(Debug, Clone)]
pub struct AgentApprovalRequest {
    pub call_id: String,
    pub tool_name: String,
    pub arguments: Value,
    pub policy: ToolExecutionPolicy,

    pub pending_tool_call: ToolCall,
    pub remaining_tool_calls: Vec<ToolCall>,
    pub accumulated_tool_results: Vec<ToolResultMessage>,

    pub resume_messages: Vec<Message>,
    pub turn_messages: Vec<AgentTurnMessage>,

    pub usage: TokenUsage,
    pub last_input_tokens: u64,
}

struct ToolCallBatchPlan {
    runnable_tool_calls: Vec<ToolCall>,
    pending_approval: Option<PendingToolApproval>,
}

struct PendingToolApproval {
    tool_call: ToolCall,
    policy: ToolExecutionPolicy,
    remaining_tool_calls: Vec<ToolCall>,
}

enum ToolCallBatchOutput {
    Completed(Vec<ToolResultMessage>),
    ApprovalRequired(AgentApprovalRequest),
}

#[derive(Debug, Clone)]
pub enum AgentProgressEvent {
    LlmThinkingStarted,
    LlmThinkingFinished,
    ToolCallRequested {
        call_id: String,
        tool_name: String,
        arguments: Value,
    },
    ToolExecutionFinished {
        call_id: String,
        tool_name: String,
        success: bool,
    },
}

pub struct AgentService<L> {
    llm_provider: L,
    tool_executor: ToolExecutor,
    model: String,
    max_tool_iterations: usize,
    system_prompt: String,
}

impl<L: LlmProvider> AgentService<L> {
    pub fn new(llm_provider: L, tool_executor: ToolExecutor) -> Self {
        Self {
            llm_provider,
            tool_executor,
            model: DEFAULT_MODEL.to_string(),
            max_tool_iterations: DEFAULT_MAX_TOOL_ITERATIONS,
            system_prompt: DEFAULT_SYSTEM_PROMPT.to_string(),
        }
    }

    pub async fn run(
        &self,
        history: Vec<Message>,
        user_message: Message,
        tx: mpsc::Sender<AgentProgressEvent>,
    ) -> Result<AgentOutput, AgentError> {
        let mut messages = vec![Message::text(Role::System, self.system_prompt.clone())];
        messages.extend(history);
        messages.push(user_message);

        self.agent_loop(messages, Vec::new(), TokenUsage::default(), tx)
            .await
    }

    async fn agent_loop(
        &self,
        mut messages: Vec<Message>,
        mut turn_messages: Vec<AgentTurnMessage>,
        mut usage: TokenUsage,
        tx: mpsc::Sender<AgentProgressEvent>,
    ) -> Result<AgentOutput, AgentError> {
        let tool_specs = self.tool_executor.specs();

        for _ in 0..self.max_tool_iterations {
            let _ = tx.send(AgentProgressEvent::LlmThinkingStarted).await;

            let response = self
                .llm_provider
                .response_with_tool(messages.clone(), tool_specs.clone(), &self.model)
                .await?;

            usage += response.usage;

            let _ = tx.send(AgentProgressEvent::LlmThinkingFinished).await;

            if response.tool_calls.is_empty() {
                let final_text = response.text;

                if !final_text.is_empty() {
                    let assistant_message = Message::text(Role::Assistant, final_text.clone());
                    messages.push(assistant_message.clone());
                    turn_messages.push(AgentTurnMessage {
                        message: assistant_message,
                        usage: Some(AgentLlmUsage {
                            model: self.model.clone(),
                            tokens: response.usage,
                        }),
                    });
                }

                return Ok(AgentOutput::Completed(AgentResult {
                    final_text,
                    messages: turn_messages,
                    usage,
                    last_input_tokens: response.usage.input_tokens,
                }));
            }

            let tool_call_message = Message::tool_call(
                if response.text.is_empty() {
                    None
                } else {
                    Some(response.text.clone())
                },
                response.tool_calls.clone(),
            );

            messages.push(tool_call_message.clone());
            turn_messages.push(AgentTurnMessage {
                message: tool_call_message,
                usage: Some(AgentLlmUsage {
                    model: self.model.clone(),
                    tokens: response.usage,
                }),
            });

            match self
                .process_tool_call_batch(
                    response.tool_calls,
                    Vec::new(),
                    &messages,
                    &turn_messages,
                    usage,
                    response.usage.input_tokens,
                    &tx,
                )
                .await
            {
                ToolCallBatchOutput::Completed(tool_results) => {
                    Self::append_tool_results(&mut messages, &mut turn_messages, tool_results);
                }
                ToolCallBatchOutput::ApprovalRequired(request) => {
                    return Ok(AgentOutput::ApprovalRequested(request));
                }
            }
        }

        Err(AgentError::MaxToolIterations(self.max_tool_iterations))
    }

    async fn process_tool_call_batch(
        &self,
        tool_calls: Vec<ToolCall>,
        mut accumulated_tool_results: Vec<ToolResultMessage>,
        resume_messages: &[Message],
        turn_messages: &[AgentTurnMessage],
        usage: TokenUsage,
        last_input_tokens: u64,
        tx: &mpsc::Sender<AgentProgressEvent>,
    ) -> ToolCallBatchOutput {
        let plan = self.plan_tool_call_batch(tool_calls);

        accumulated_tool_results
            .extend(self.parallel_tool_calls(plan.runnable_tool_calls, tx).await);

        match plan.pending_approval {
            Some(pending) => ToolCallBatchOutput::ApprovalRequired(self.build_approval_request(
                pending,
                accumulated_tool_results,
                resume_messages,
                turn_messages,
                usage,
                last_input_tokens,
            )),
            None => ToolCallBatchOutput::Completed(accumulated_tool_results),
        }
    }

    fn build_approval_request(
        &self,
        pending: PendingToolApproval,
        accumulated_tool_results: Vec<ToolResultMessage>,
        resume_messages: &[Message],
        turn_messages: &[AgentTurnMessage],
        usage: TokenUsage,
        last_input_tokens: u64,
    ) -> AgentApprovalRequest {
        AgentApprovalRequest {
            call_id: pending.tool_call.id.clone(),
            tool_name: pending.tool_call.name.clone(),
            arguments: pending.tool_call.arguments.clone(),
            policy: pending.policy,
            pending_tool_call: pending.tool_call,
            remaining_tool_calls: pending.remaining_tool_calls,
            accumulated_tool_results,
            resume_messages: resume_messages.to_vec(),
            turn_messages: turn_messages.to_vec(),
            usage,
            last_input_tokens,
        }
    }

    fn append_tool_results(
        messages: &mut Vec<Message>,
        turn_messages: &mut Vec<AgentTurnMessage>,
        tool_results: Vec<ToolResultMessage>,
    ) {
        let tool_result_message = Message::tool_results(tool_results);

        messages.push(tool_result_message.clone());
        turn_messages.push(AgentTurnMessage {
            message: tool_result_message,
            usage: None,
        });
    }

    /// Executes all tool calls in parallel and sends progress events.
    async fn parallel_tool_calls(
        &self,
        tool_calls: Vec<ToolCall>,
        tx: &mpsc::Sender<AgentProgressEvent>,
    ) -> Vec<ToolResultMessage> {
        // 1. send ToolCallRequested for all calls
        for call in &tool_calls {
            let _ = tx
                .send(AgentProgressEvent::ToolCallRequested {
                    call_id: call.id.clone(),
                    tool_name: call.name.clone(),
                    arguments: call.arguments.clone(),
                })
                .await;
        }

        // 2. execute all tool calls in parallel
        let raw_results = join_all(tool_calls.into_iter().map(|call| async move {
            let call_id = call.id.clone();
            let tool_name = call.name.clone();
            let result = self.tool_executor.execute(call).await;
            (call_id, tool_name, result)
        }))
        .await;

        let mut tool_results = Vec::with_capacity(raw_results.len());

        // 3. send ToolExecutionFinished for all calls
        for (call_id, tool_name, result) in raw_results {
            let tool_result = match result {
                Ok(result) => ToolResultMessage::from_execution(call_id.clone(), result),
                Err(err) => ToolResultMessage::from_execution(
                    call_id.clone(),
                    ToolExecutionResult::error(json!({
                        "message": err.to_string()
                    })),
                ),
            };

            let _ = tx
                .send(AgentProgressEvent::ToolExecutionFinished {
                    call_id,
                    tool_name,
                    success: !tool_result.is_error,
                })
                .await;

            tool_results.push(tool_result);
        }

        tool_results
    }

    fn plan_tool_call_batch(&self, tool_calls: Vec<ToolCall>) -> ToolCallBatchPlan {
        let Some((approval_index, policy)) = self.first_approval_required_tool_call(&tool_calls)
        else {
            return ToolCallBatchPlan {
                runnable_tool_calls: tool_calls,
                pending_approval: None,
            };
        };

        let pending_tool_call = tool_calls[approval_index].clone();

        ToolCallBatchPlan {
            runnable_tool_calls: tool_calls[..approval_index].to_vec(),
            pending_approval: Some(PendingToolApproval {
                tool_call: pending_tool_call,
                policy,
                remaining_tool_calls: tool_calls[approval_index + 1..].to_vec(),
            }),
        }
    }

    fn first_approval_required_tool_call(
        &self,
        tool_calls: &[ToolCall],
    ) -> Option<(usize, ToolExecutionPolicy)> {
        tool_calls.iter().enumerate().find_map(|(index, call)| {
            let policy = self.tool_executor.check_execution_policy(call).ok()?;

            if matches!(
                policy,
                ToolExecutionPolicy::Ask | ToolExecutionPolicy::ConfirmEveryTime
            ) {
                Some((index, policy))
            } else {
                None
            }
        })
    }

    pub async fn resume_after_approval(
        &self,
        request: AgentApprovalRequest,
        tx: mpsc::Sender<AgentProgressEvent>,
    ) -> Result<AgentOutput, AgentError> {
        let AgentApprovalRequest {
            pending_tool_call,
            remaining_tool_calls,
            mut accumulated_tool_results,
            mut resume_messages,
            mut turn_messages,
            usage,
            last_input_tokens,
            ..
        } = request;

        accumulated_tool_results
            .extend(self.parallel_tool_calls(vec![pending_tool_call], &tx).await);

        match self
            .process_tool_call_batch(
                remaining_tool_calls,
                accumulated_tool_results,
                &resume_messages,
                &turn_messages,
                usage,
                last_input_tokens,
                &tx,
            )
            .await
        {
            ToolCallBatchOutput::Completed(tool_results) => {
                Self::append_tool_results(&mut resume_messages, &mut turn_messages, tool_results);
                self.agent_loop(resume_messages, turn_messages, usage, tx)
                    .await
            }
            ToolCallBatchOutput::ApprovalRequired(request) => {
                Ok(AgentOutput::ApprovalRequested(request))
            }
        }
    }
}
