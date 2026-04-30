use crate::domain::error::agent_error::AgentError;
use crate::domain::model::message::{Message, MessageContent};
use crate::domain::model::role::Role;
use crate::domain::model::token_usage::TokenUsage;
use crate::domain::model::tool_approval::{ToolApprovalDecision, ToolApprovalRequest};
use crate::domain::model::tool_call::{ToolCall, ToolCallOutput, ToolCallOutputStatus};
use crate::domain::model::tool_execution_decision::ToolExecutionDecision;
use crate::domain::port::llm_provider::LlmProvider;
use crate::domain::service::tool_service::ToolService;
use serde_json::json;
use tokio::sync::mpsc;

const DEFAULT_MODEL: &str = "global.anthropic.claude-sonnet-4-6";
const DEFAULT_MAX_TOOL_ITERATIONS: usize = 20;

#[derive(Debug, Clone)]
pub struct AgentCompletion {
    pub messages: Vec<Message>,
    pub usage: TokenUsage,
}

#[derive(Debug, Clone)]
pub enum AgentOutput {
    Completed(AgentCompletion),
    ApprovalRequired(Box<AgentApprovalRequired>),
}

#[derive(Debug, Clone)]
pub struct AgentApprovalRequired {
    pub request: ToolApprovalRequest,
    pub continuation: AgentContinuation,
}

#[derive(Debug, Clone)]
pub struct AgentContinuation {
    input_messages: Vec<Message>,
    new_messages: Vec<Message>,
    usage: TokenUsage,

    pending_tool_call: ToolCall,
    remaining_tool_calls: Vec<ToolCall>,
    accumulated_tool_results: Vec<ToolCallOutput>,
}

#[derive(Debug, Clone)]
pub enum AgentEvent {
    LlmStarted,
    LlmFinished,

    ToolStarted {
        call_id: String,
        tool_name: String,
    },
    ToolFinished {
        call_id: String,
        tool_name: String,
        success: bool,
    },
}

pub struct AgentService<L> {
    llm_provider: L,
    tool_service: ToolService,
    model: String,
    max_tool_iterations: usize,
}

impl<L: LlmProvider> AgentService<L> {
    pub fn new(llm_provider: L, tool_service: ToolService) -> Self {
        Self {
            llm_provider,
            tool_service,
            model: DEFAULT_MODEL.to_string(),
            max_tool_iterations: DEFAULT_MAX_TOOL_ITERATIONS,
        }
    }

    pub fn tool_service(&self) -> &ToolService {
        &self.tool_service
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    async fn agent_loop(
        &self,
        input_messages: Vec<Message>,
        mut new_messages: Vec<Message>,
        mut usage: TokenUsage,
        tx: mpsc::Sender<AgentEvent>,
    ) -> Result<AgentOutput, AgentError> {
        for _ in 0..self.max_tool_iterations {
            // 1. Build the LLM input from the initial messages and the messages produced so far.
            let mut llm_messages = input_messages.clone();
            llm_messages.extend(new_messages.clone());

            // 2. Call the LLM with the available tool specs.
            let _ = tx.send(AgentEvent::LlmStarted).await;
            let response = self
                .llm_provider
                .response_with_tool(llm_messages, self.tool_service.specs(), &self.model)
                .await?;
            usage += response.usage;
            let _ = tx.send(AgentEvent::LlmFinished).await;

            // 3. Add the assistant response, including tool calls when present.
            if !response.text.is_empty() || response.tool_calls.is_empty() {
                new_messages.push(Message::output_text(response.text.clone())?);
            }

            if !response.tool_calls.is_empty() {
                new_messages.push(Message::tool_calls(response.tool_calls.clone())?);
            }

            // 4. Complete the run when the LLM did not request any tools.
            if response.tool_calls.is_empty() {
                return Ok(AgentOutput::Completed(AgentCompletion {
                    messages: new_messages,
                    usage,
                }));
            }

            // 5. Execute, block, or pause for approval for each requested tool call.
            let tool_calls = response.tool_calls;
            let mut tool_call_results = Vec::new();
            for (index, tool_call) in tool_calls.iter().cloned().enumerate() {
                // 5.1 Decide whether this tool call can run, must ask, or should be blocked.
                match self.tool_service.decide_execution(&tool_call).await {
                    Ok(ToolExecutionDecision::Allow) => {
                        // 5.2 Run the tool and collect its result for the LLM.
                        let call_id = tool_call.call_id.clone();
                        let tool_name = tool_call.name.clone();

                        let _ = tx
                            .send(AgentEvent::ToolStarted {
                                call_id: call_id.clone(),
                                tool_name: tool_name.clone(),
                            })
                            .await;

                        let result = self.tool_service.execute(tool_call).await;

                        let tool_result = match result {
                            Ok(result) => result,
                            Err(err) => ToolCallOutput::error(
                                call_id.clone(),
                                json!({
                                    "message": err.to_string(),
                                }),
                            ),
                        };

                        let _ = tx
                            .send(AgentEvent::ToolFinished {
                                call_id,
                                tool_name,
                                success: tool_result.status == ToolCallOutputStatus::Success,
                            })
                            .await;

                        tool_call_results.push(tool_result);
                    }
                    Ok(ToolExecutionDecision::Ask) => {
                        // 5.3 Pause the run and return everything needed to resume after approval.
                        let policy = self.tool_service.check_execution_policy(&tool_call)?;
                        let remaining_tool_calls = tool_calls[index + 1..].to_vec();
                        return Ok(AgentOutput::ApprovalRequired(Box::new(
                            AgentApprovalRequired {
                                request: ToolApprovalRequest {
                                    call_id: tool_call.call_id.clone(),
                                    tool_name: tool_call.name.clone(),
                                    arguments: tool_call.arguments.clone(),
                                    policy,
                                },
                                continuation: AgentContinuation {
                                    input_messages,
                                    new_messages,
                                    usage,
                                    pending_tool_call: tool_call,
                                    remaining_tool_calls,
                                    accumulated_tool_results: tool_call_results,
                                },
                            },
                        )));
                    }
                    Ok(ToolExecutionDecision::Deny) => {
                        // 5.4 Convert a blocked tool call into a tool result for the LLM.
                        tool_call_results.push(ToolCallOutput::error(
                            tool_call.call_id,
                            json!({
                                "message": "tool execution was blocked by execution rule",
                            }),
                        ));
                    }
                    Err(err) => {
                        // 5.5 Convert tool lookup or policy errors into a tool result for the LLM.
                        tool_call_results.push(ToolCallOutput::error(
                            tool_call.call_id,
                            json!({
                                "message": err.to_string(),
                            }),
                        ));
                    }
                }
            }

            // 6. Feed all tool results back into the next LLM iteration.
            if !tool_call_results.is_empty() {
                new_messages.push(Message::tool_call_outputs(tool_call_results)?);
            }
        }

        // 7. Stop when tool execution keeps cycling beyond the configured limit.
        Err(AgentError::MaxToolIterations(self.max_tool_iterations))
    }

    pub async fn run(
        &self,
        instruction: String,
        messages: Vec<Message>,
        tx: mpsc::Sender<AgentEvent>,
    ) -> Result<AgentOutput, AgentError> {
        let instructions =
            Message::new(Role::System, vec![MessageContent::InputText(instruction)])?;
        let input_messages = {
            let mut all_messages = Vec::with_capacity(messages.len() + 1);
            all_messages.push(instructions);
            all_messages.extend(messages);
            all_messages
        };

        let new_messages: Vec<Message> = Vec::new();
        let usage = TokenUsage::default();

        // agent loop
        self.agent_loop(input_messages, new_messages, usage, tx)
            .await
    }

    pub async fn resume(
        &self,
        continuation: AgentContinuation,
        decision: ToolApprovalDecision,
        tx: mpsc::Sender<AgentEvent>,
    ) -> Result<AgentOutput, AgentError> {
        // 1. Restore the paused run state.
        let AgentContinuation {
            input_messages,
            mut new_messages,
            usage,
            pending_tool_call,
            remaining_tool_calls,
            mut accumulated_tool_results,
        } = continuation;

        // 2. Apply the user's decision to the pending tool call.
        match decision {
            ToolApprovalDecision::Approved => {
                // 2.1 Run the approved tool and collect its result for the LLM.
                let call_id = pending_tool_call.call_id.clone();
                let tool_name = pending_tool_call.name.clone();

                let _ = tx
                    .send(AgentEvent::ToolStarted {
                        call_id: call_id.clone(),
                        tool_name: tool_name.clone(),
                    })
                    .await;

                let result = self.tool_service.execute(pending_tool_call).await;

                let tool_result = match result {
                    Ok(result) => result,
                    Err(err) => ToolCallOutput::error(
                        call_id.clone(),
                        json!({
                            "message": err.to_string(),
                        }),
                    ),
                };

                let _ = tx
                    .send(AgentEvent::ToolFinished {
                        call_id,
                        tool_name,
                        success: tool_result.status == ToolCallOutputStatus::Success,
                    })
                    .await;

                accumulated_tool_results.push(tool_result);
            }
            ToolApprovalDecision::Denied => {
                // 2.2 Convert the denied tool call into a tool result for the LLM.
                accumulated_tool_results.push(ToolCallOutput::error(
                    pending_tool_call.call_id,
                    json!({
                        "message": "tool execution was denied by user",
                    }),
                ));
            }
        }

        let mut remaining_tool_calls = remaining_tool_calls;

        // 3. Continue the remaining tool calls from the same assistant response.
        while !remaining_tool_calls.is_empty() {
            let tool_call = remaining_tool_calls.remove(0);

            // 3.1 Decide whether this tool call can run, must ask, or should be blocked.
            match self.tool_service.decide_execution(&tool_call).await {
                Ok(ToolExecutionDecision::Allow) => {
                    // 3.2 Run the tool and collect its result for the LLM.
                    let call_id = tool_call.call_id.clone();
                    let tool_name = tool_call.name.clone();

                    let _ = tx
                        .send(AgentEvent::ToolStarted {
                            call_id: call_id.clone(),
                            tool_name: tool_name.clone(),
                        })
                        .await;

                    let result = self.tool_service.execute(tool_call).await;

                    let tool_result = match result {
                        Ok(result) => result,
                        Err(err) => ToolCallOutput::error(
                            call_id.clone(),
                            json!({
                                "message": err.to_string(),
                            }),
                        ),
                    };

                    let _ = tx
                        .send(AgentEvent::ToolFinished {
                            call_id,
                            tool_name,
                            success: tool_result.status == ToolCallOutputStatus::Success,
                        })
                        .await;

                    accumulated_tool_results.push(tool_result);
                }
                Ok(ToolExecutionDecision::Ask) => {
                    // 3.3 Pause again and return everything needed to resume after approval.
                    let policy = self.tool_service.check_execution_policy(&tool_call)?;

                    return Ok(AgentOutput::ApprovalRequired(Box::new(
                        AgentApprovalRequired {
                            request: ToolApprovalRequest {
                                call_id: tool_call.call_id.clone(),
                                tool_name: tool_call.name.clone(),
                                arguments: tool_call.arguments.clone(),
                                policy,
                            },
                            continuation: AgentContinuation {
                                input_messages,
                                new_messages,
                                usage,
                                pending_tool_call: tool_call,
                                remaining_tool_calls,
                                accumulated_tool_results,
                            },
                        },
                    )));
                }
                Ok(ToolExecutionDecision::Deny) => {
                    // 3.4 Convert a blocked tool call into a tool result for the LLM.
                    accumulated_tool_results.push(ToolCallOutput::error(
                        tool_call.call_id,
                        json!({
                            "message": "tool execution was blocked by execution rule",
                        }),
                    ));
                }
                Err(err) => {
                    accumulated_tool_results.push(ToolCallOutput::error(
                        tool_call.call_id,
                        json!({
                            "message": err.to_string(),
                        }),
                    ));
                }
            }
        }

        // 4. Feed all accumulated tool results back into the next LLM iteration.
        if !accumulated_tool_results.is_empty() {
            new_messages.push(Message::tool_call_outputs(accumulated_tool_results)?);
        }

        // 5. Continue the normal agent loop.
        self.agent_loop(input_messages, new_messages, usage, tx)
            .await
    }
}
