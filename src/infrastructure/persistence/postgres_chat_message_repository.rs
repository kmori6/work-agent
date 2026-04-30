use crate::domain::error::chat_repository_error::ChatRepositoryError;
use crate::domain::model::chat_message::ChatMessage;
use crate::domain::model::message::{Message, MessageContent, MessageType};
use crate::domain::model::role::Role;
use crate::domain::model::tool_call::{ToolCall, ToolCallOutput, ToolCallOutputStatus};
use crate::domain::repository::chat_message_repository::{
    ChatMessageRepository, ChatMessageSummary,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;
use std::collections::HashMap;
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct ChatMessageSummaryRow {
    session_id: Uuid,
    first_user_message: Option<String>,
    message_count: i64,
}

impl From<ChatMessageSummaryRow> for ChatMessageSummary {
    fn from(row: ChatMessageSummaryRow) -> Self {
        Self {
            session_id: row.session_id,
            first_user_message: row.first_user_message,
            message_count: row.message_count,
        }
    }
}

#[derive(sqlx::FromRow)]
struct ChatMessageRow {
    id: Uuid,
    session_id: Uuid,
    role: String,
    created_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct ChatMessageContentRow {
    message_id: Uuid,
    content_type: String,
    text: Option<String>,
    call_id: Option<String>,
    tool_name: Option<String>,
    arguments: Option<Value>,
    output: Option<Value>,
    result_status: Option<String>,
}

fn map_sqlx_error(err: sqlx::Error) -> ChatRepositoryError {
    ChatRepositoryError::Unexpected(err.to_string())
}

fn role_to_db(role: Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
    }
}

fn role_from_db(value: &str) -> Result<Role, ChatRepositoryError> {
    match value {
        "system" => Ok(Role::System),
        "user" => Ok(Role::User),
        "assistant" => Ok(Role::Assistant),
        _ => Err(ChatRepositoryError::Unexpected(format!(
            "unknown role: {value}"
        ))),
    }
}

fn message_type_to_db(message_type: MessageType) -> &'static str {
    match message_type {
        MessageType::Message => "message",
        MessageType::Tool => "tool",
    }
}

fn status_to_db(status: ToolCallOutputStatus) -> &'static str {
    match status {
        ToolCallOutputStatus::Success => "success",
        ToolCallOutputStatus::Error => "error",
    }
}

fn status_from_db(value: Option<&str>) -> ToolCallOutputStatus {
    match value {
        Some("error") => ToolCallOutputStatus::Error,
        _ => ToolCallOutputStatus::Success,
    }
}

fn content_row_to_message_content(
    row: ChatMessageContentRow,
) -> Result<MessageContent, ChatRepositoryError> {
    match row.content_type.as_str() {
        "input_text" => Ok(MessageContent::InputText(row.text.unwrap_or_default())),
        "output_text" => Ok(MessageContent::OutputText(row.text.unwrap_or_default())),
        "tool_call" => Ok(MessageContent::ToolCall(ToolCall {
            call_id: row.call_id.unwrap_or_default(),
            name: row.tool_name.unwrap_or_default(),
            arguments: row.arguments.unwrap_or(Value::Null),
        })),
        "tool_call_output" => Ok(MessageContent::ToolCallOutput(ToolCallOutput {
            call_id: row.call_id.unwrap_or_default(),
            output: row.output.unwrap_or(Value::Null),
            status: status_from_db(row.result_status.as_deref()),
        })),
        other => Err(ChatRepositoryError::Unexpected(format!(
            "unknown message content type: {other}"
        ))),
    }
}

#[derive(Clone)]
pub struct PostgresChatMessageRepository {
    pool: PgPool,
}

impl PostgresChatMessageRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ChatMessageRepository for PostgresChatMessageRepository {
    async fn append(
        &self,
        session_id: Uuid,
        message: Message,
    ) -> Result<ChatMessage, ChatRepositoryError> {
        let message_type = message
            .message_type()
            .map_err(|err| ChatRepositoryError::Unexpected(err.to_string()))?;

        let role = role_to_db(message.role).to_string();
        let message_type = message_type_to_db(message_type).to_string();

        // transaction: 1. update chat_sessions.updated_at -> 2. insert into chat_messages
        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;

        let updated = sqlx::query_scalar::<_, Uuid>(
            r#"
        UPDATE chat_sessions
        SET updated_at = NOW()
        WHERE id = $1
        RETURNING id
        "#,
        )
        .bind(session_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;

        if updated.is_none() {
            return Err(ChatRepositoryError::SessionNotFound(session_id));
        }

        let row = sqlx::query_as::<_, ChatMessageRow>(
            r#"
            INSERT INTO chat_messages (session_id, role, type)
            VALUES ($1, $2, $3)
            RETURNING id, session_id, role, created_at
            "#,
        )
        .bind(session_id)
        .bind(role)
        .bind(message_type)
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;

        for (content_index, content) in message.contents.iter().enumerate() {
            if !content.is_persistable() {
                continue;
            }

            let (content_type, text, call_id, tool_name, arguments, output, result_status) =
                match content {
                    MessageContent::InputText(text) => (
                        "input_text",
                        Some(text.clone()),
                        None,
                        None,
                        None,
                        None,
                        None,
                    ),
                    MessageContent::OutputText(text) => (
                        "output_text",
                        Some(text.clone()),
                        None,
                        None,
                        None,
                        None,
                        None,
                    ),
                    MessageContent::ToolCall(call) => (
                        "tool_call",
                        None,
                        Some(call.call_id.clone()),
                        Some(call.name.clone()),
                        Some(call.arguments.clone()),
                        None,
                        None,
                    ),
                    MessageContent::ToolCallOutput(output) => (
                        "tool_call_output",
                        None,
                        Some(output.call_id.clone()),
                        None,
                        None,
                        Some(output.output.clone()),
                        Some(status_to_db(output.status).to_string()),
                    ),
                    MessageContent::InputImage(_) | MessageContent::InputFile(_) => continue,
                };

            sqlx::query(
                r#"
            INSERT INTO chat_message_contents (
                message_id,
                content_index,
                type,
                text,
                call_id,
                tool_name,
                arguments,
                output,
                result_status
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            "#,
            )
            .bind(row.id)
            .bind(content_index as i32)
            .bind(content_type)
            .bind(text)
            .bind(call_id)
            .bind(tool_name)
            .bind(arguments)
            .bind(output)
            .bind(result_status)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx_error)?;
        }

        tx.commit().await.map_err(map_sqlx_error)?;

        Ok(ChatMessage {
            id: row.id,
            session_id: row.session_id,
            message,
            created_at: row.created_at,
        })
    }

    async fn list_for_session(
        &self,
        session_id: Uuid,
    ) -> Result<Vec<ChatMessage>, ChatRepositoryError> {
        let message_rows = sqlx::query_as::<_, ChatMessageRow>(
            r#"
            SELECT id, session_id, role, created_at
            FROM chat_messages
            WHERE session_id = $1
            ORDER BY id ASC
            "#,
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        if message_rows.is_empty() {
            return Ok(Vec::new());
        }

        let message_ids = message_rows.iter().map(|row| row.id).collect::<Vec<_>>();

        let content_rows = sqlx::query_as::<_, ChatMessageContentRow>(
            r#"
            SELECT
                message_id,
                content_index,
                type AS content_type,
                text,
                call_id,
                tool_name,
                arguments,
                output,
                result_status
            FROM chat_message_contents
            WHERE message_id = ANY($1::uuid[])
            ORDER BY message_id ASC, content_index ASC
            "#,
        )
        .bind(message_ids)
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        let mut contents_by_message_id = HashMap::new();

        for row in content_rows {
            contents_by_message_id
                .entry(row.message_id)
                .or_insert_with(Vec::new)
                .push(row);
        }

        message_rows
            .into_iter()
            .map(|row| {
                let role = role_from_db(&row.role)?;

                let contents = contents_by_message_id
                    .remove(&row.id)
                    .unwrap_or_default()
                    .into_iter()
                    .map(content_row_to_message_content)
                    .collect::<Result<Vec<_>, _>>()?;

                let message = Message::new(role, contents)
                    .map_err(|err| ChatRepositoryError::Unexpected(err.to_string()))?;

                Ok(ChatMessage {
                    id: row.id,
                    session_id: row.session_id,
                    message,
                    created_at: row.created_at,
                })
            })
            .collect()
    }

    async fn summarize_by_session_ids(
        &self,
        session_ids: &[Uuid],
    ) -> Result<Vec<ChatMessageSummary>, ChatRepositoryError> {
        if session_ids.is_empty() {
            return Ok(Vec::new());
        }

        let rows = sqlx::query_as::<_, ChatMessageSummaryRow>(
            r#"
            SELECT
                counts.session_id,
                first_user_message.text AS first_user_message,
                counts.message_count
            FROM (
                SELECT
                    session_id,
                    COUNT(*) AS message_count
                FROM chat_messages
                WHERE session_id = ANY($1::uuid[])
                GROUP BY session_id
            ) counts
            LEFT JOIN LATERAL (
                SELECT cmc.text
                FROM chat_messages cm
                JOIN chat_message_contents cmc ON cmc.message_id = cm.id
                WHERE cm.session_id = counts.session_id
                AND cm.role = 'user'
                AND cm.type = 'message'
                AND cmc.type = 'input_text'
                AND cmc.text IS NOT NULL
                ORDER BY cm.id ASC, cmc.content_index ASC
                LIMIT 1
            ) first_user_message ON true
            "#,
        )
        .bind(session_ids.to_vec())
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        Ok(rows.into_iter().map(Into::into).collect())
    }
}
