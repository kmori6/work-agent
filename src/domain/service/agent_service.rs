use crate::domain::error::agent_error::AgentError;
use crate::domain::model::message::Message;
use crate::domain::model::role::Role;
use crate::domain::model::token_usage::TokenUsage;
use crate::domain::model::tool::ToolResultMessage;
use crate::domain::port::llm_provider::LlmProvider;
use crate::domain::port::llm_provider::LlmResponse;
use crate::domain::service::tool_service::ToolExecutor;
use futures::future::join_all;
use serde_json::Value;
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
    ) -> Result<AgentResult, AgentError> {
        let mut messages = vec![Message::text(Role::System, self.system_prompt.clone())];
        messages.extend(history);
        messages.push(user_message);

        let mut turn_messages = Vec::new();
        let tool_specs = self.tool_executor.specs();

        let mut usage = TokenUsage::default();

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

                return Ok(AgentResult {
                    final_text,
                    messages: turn_messages,
                    usage,
                    last_input_tokens: response.usage.input_tokens,
                });
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

            let tool_results = self.parallel_tool_calls(response, &tx).await;
            let tool_result_message = Message::tool_results(tool_results);
            messages.push(tool_result_message.clone());
            turn_messages.push(AgentTurnMessage {
                message: tool_result_message,
                usage: None,
            });
        }

        Err(AgentError::MaxToolIterations(self.max_tool_iterations))
    }

    /// Executes all tool calls in parallel and sends progress events.
    async fn parallel_tool_calls(
        &self,
        response: LlmResponse,
        tx: &mpsc::Sender<AgentProgressEvent>,
    ) -> Vec<ToolResultMessage> {
        // 1. send ToolCallRequested for all calls
        let mut call_metadata = Vec::with_capacity(response.tool_calls.len());
        for call in &response.tool_calls {
            let _ = tx
                .send(AgentProgressEvent::ToolCallRequested {
                    call_id: call.id.clone(),
                    tool_name: call.name.clone(),
                    arguments: call.arguments.clone(),
                })
                .await;
            call_metadata.push((call.id.clone(), call.name.clone()));
        }

        // 2. execute all tool calls in parallel
        let raw_results = join_all(
            response
                .tool_calls
                .into_iter()
                .map(|call| self.tool_executor.execute(call)),
        )
        .await;

        // 3. send ToolExecutionFinished for all calls
        let mut tool_results = Vec::with_capacity(raw_results.len());
        for ((call_id, tool_name), result) in call_metadata.into_iter().zip(raw_results) {
            let _ = tx
                .send(AgentProgressEvent::ToolExecutionFinished {
                    call_id,
                    tool_name,
                    success: !result.is_error,
                })
                .await;
            tool_results.push(result);
        }

        tool_results
    }
}
