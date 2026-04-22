use crate::domain::error::agent_error::AgentError;
use crate::domain::model::message::{Message, MessageContent};
use crate::domain::model::role::Role;
use crate::domain::model::token_usage::TokenUsage;
use crate::domain::port::llm_provider::LlmProvider;

const DEFAULT_MODEL: &str = "global.anthropic.claude-sonnet-4-6";
const DEFAULT_CONTEXT_WINDOW_TOKENS: u64 = 1_000_000;
const DEFAULT_COMPACTION_THRESHOLD_PERCENT: u64 = 80;
const RECENT_MESSAGES_TO_KEEP: usize = 8;

pub struct ContextService<L> {
    llm_provider: L,
    model: String,
    context_window_tokens: u64,
    compaction_threshold_percent: u64,
}

impl<L: LlmProvider> ContextService<L> {
    pub fn new(llm_provider: L) -> Self {
        Self {
            llm_provider,
            model: DEFAULT_MODEL.to_string(),
            context_window_tokens: DEFAULT_CONTEXT_WINDOW_TOKENS,
            compaction_threshold_percent: DEFAULT_COMPACTION_THRESHOLD_PERCENT,
        }
    }

    pub fn with_config(
        llm_provider: L,
        model: impl Into<String>,
        context_window_tokens: u64,
        compaction_threshold_percent: u64,
    ) -> Self {
        Self {
            llm_provider,
            model: model.into(),
            context_window_tokens,
            compaction_threshold_percent,
        }
    }

    pub async fn build_context(
        &self,
        history: Vec<Message>,
        latest_usage: Option<TokenUsage>,
    ) -> Result<Vec<Message>, AgentError> {
        let input_tokens = latest_usage.map_or(0, |usage| usage.input_tokens);

        if !self.should_compact(input_tokens) {
            return Ok(history);
        }

        self.compact_messages(history).await
    }

    pub fn context_window_tokens(&self) -> u64 {
        self.context_window_tokens
    }

    pub fn percent_used(&self, input_tokens: u64) -> u64 {
        if self.context_window_tokens == 0 {
            return 0;
        }

        input_tokens.saturating_mul(100) / self.context_window_tokens
    }

    fn should_compact(&self, input_tokens: u64) -> bool {
        self.percent_used(input_tokens) >= self.compaction_threshold_percent
    }

    async fn compact_messages(&self, history: Vec<Message>) -> Result<Vec<Message>, AgentError> {
        let split_at = history.len().saturating_sub(RECENT_MESSAGES_TO_KEEP);
        let old_messages = history[..split_at].to_vec();
        let recent_messages = history[split_at..].to_vec();

        if old_messages.is_empty() {
            return Ok(recent_messages);
        }

        let old_text = old_messages
            .iter()
            .map(format_message_for_summary)
            .collect::<Vec<_>>()
            .join("\n\n");

        let summary_prompt = format!(
            "Summarize the following conversation history for future turns.\n\
Keep only facts, decisions, unresolved tasks, important file names, user preferences, and implementation direction.\n\
Do not add guesses or new conclusions.\n\n\
Conversation history:\n{old_text}"
        );

        let summary = self
            .llm_provider
            .response(vec![Message::text(Role::User, summary_prompt)], &self.model)
            .await?;

        let mut compacted = Vec::with_capacity(recent_messages.len() + 1);
        compacted.push(Message::text(
            Role::Assistant,
            format!("Conversation summary so far:\n{summary}"),
        ));
        compacted.extend(recent_messages);

        Ok(compacted)
    }
}

fn format_message_for_summary(message: &Message) -> String {
    format!(
        "{:?}: {}",
        message.role,
        format_message_content_for_summary(message)
    )
}

fn format_message_content_for_summary(message: &Message) -> String {
    match &message.content {
        MessageContent::Text(text) => text.clone(),

        MessageContent::Multimodal { text, attachments } => {
            let filenames = attachments
                .iter()
                .map(|attachment| attachment.filename.as_str())
                .collect::<Vec<_>>()
                .join(", ");

            if filenames.is_empty() {
                text.clone()
            } else {
                format!("{text}\nattachments: {filenames}")
            }
        }

        MessageContent::ToolCall { text, tool_calls } => {
            let tool_names = tool_calls
                .iter()
                .map(|call| call.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");

            match text {
                Some(text) if !tool_names.is_empty() => {
                    format!("{text}\ntool_calls: {tool_names}")
                }
                Some(text) => text.clone(),
                None => format!("tool_calls: {tool_names}"),
            }
        }

        MessageContent::ToolResults(tool_results) => {
            format!("tool_results: {} result(s)", tool_results.len())
        }
    }
}
