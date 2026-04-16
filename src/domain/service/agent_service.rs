use crate::domain::error::agent_error::AgentError;
use crate::domain::model::message::Message;
use crate::domain::model::role::Role;
use crate::domain::port::llm_provider::LlmProvider;
use crate::domain::service::tool_service::ToolExecutor;
use serde_json::Value;

const DEFAULT_MODEL: &str = "global.anthropic.claude-sonnet-4-6";
const DEFAULT_MAX_TOOL_ITERATIONS: usize = 20;
const DEFAULT_SYSTEM_PROMPT: &str = "\
You are a helpful assistant.
Use the web_search tool when the user asks for recent, current, or web-based information.
After receiving tool results, answer clearly in Japanese.
";

#[derive(Debug, Clone)]
pub struct AgentResult {
    pub final_text: String,
    pub messages: Vec<Message>,
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

    pub fn with_config(
        llm_provider: L,
        tool_executor: ToolExecutor,
        model: impl Into<String>,
        system_prompt: impl Into<String>,
        max_tool_iterations: usize,
    ) -> Self {
        Self {
            llm_provider,
            tool_executor,
            model: model.into(),
            max_tool_iterations,
            system_prompt: system_prompt.into(),
        }
    }

    pub async fn run(&self, user_input: impl Into<String>) -> Result<AgentResult, AgentError> {
        let mut messages = vec![
            Message::text(Role::System, self.system_prompt.clone()),
            Message::text(Role::User, user_input.into()),
        ];

        let tool_specs = self.tool_executor.specs();

        for _ in 0..self.max_tool_iterations {
            let response = self
                .llm_provider
                .response_with_tool(messages.clone(), tool_specs.clone(), &self.model)
                .await?;

            if response.tool_calls.is_empty() {
                let final_text = response.text;

                if !final_text.is_empty() {
                    messages.push(Message::text(Role::Assistant, final_text.clone()));
                }

                return Ok(AgentResult {
                    final_text,
                    messages,
                });
            }

            messages.push(Message::tool_call(
                if response.text.is_empty() {
                    None
                } else {
                    Some(response.text.clone())
                },
                response.tool_calls.clone(),
            ));

            let mut tool_results = Vec::with_capacity(response.tool_calls.len());

            for call in response.tool_calls {
                let result = self.tool_executor.execute(call).await;
                tool_results.push(result);
            }

            messages.push(Message::tool_results(tool_results));
        }

        Err(AgentError::MaxToolIterations(self.max_tool_iterations))
    }

    pub async fn run_with_progress<F>(
        &self,
        user_input: impl Into<String>,
        mut emit: F,
    ) -> Result<AgentResult, AgentError>
    where
        F: FnMut(AgentProgressEvent),
    {
        let mut messages = vec![
            Message::text(Role::System, self.system_prompt.clone()),
            Message::text(Role::User, user_input.into()),
        ];

        let tool_specs = self.tool_executor.specs();

        for _ in 0..self.max_tool_iterations {
            emit(AgentProgressEvent::LlmThinkingStarted);

            let response = self
                .llm_provider
                .response_with_tool(messages.clone(), tool_specs.clone(), &self.model)
                .await?;

            emit(AgentProgressEvent::LlmThinkingFinished);

            if response.tool_calls.is_empty() {
                let final_text = response.text;

                if !final_text.is_empty() {
                    messages.push(Message::text(Role::Assistant, final_text.clone()));
                }

                return Ok(AgentResult {
                    final_text,
                    messages,
                });
            }

            messages.push(Message::tool_call(
                if response.text.is_empty() {
                    None
                } else {
                    Some(response.text.clone())
                },
                response.tool_calls.clone(),
            ));

            let mut tool_results = Vec::with_capacity(response.tool_calls.len());

            for call in response.tool_calls {
                emit(AgentProgressEvent::ToolCallRequested {
                    call_id: call.id.clone(),
                    tool_name: call.name.clone(),
                    arguments: call.arguments.clone(),
                });

                let call_id = call.id.clone();
                let tool_name = call.name.clone();

                let result = self.tool_executor.execute(call).await;

                let success = !result.is_error;

                emit(AgentProgressEvent::ToolExecutionFinished {
                    call_id,
                    tool_name,
                    success,
                });

                tool_results.push(result);
            }

            messages.push(Message::tool_results(tool_results));
        }

        Err(AgentError::MaxToolIterations(self.max_tool_iterations))
    }
}
