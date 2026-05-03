use crate::domain::model::chat_session::ChatSession;
use crate::presentation::error::agent_cli_error::AgentCliError;
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use serde_json::{Value, json};
use uuid::Uuid;

const PROMPT: &str = "\x1b[38;2;0;71;171m>\x1b[0m ";
const MAX_CHARS: usize = 100;

struct ChatApiClient {
    base_url: String,
    http: reqwest::Client,
}

impl ChatApiClient {
    fn new(base_url: String) -> Self {
        Self {
            // http://localhost:3000/ -> http://localhost:3000
            base_url: base_url.trim_end_matches('/').to_string(),
            http: reqwest::Client::new(),
        }
    }

    async fn health(&self) -> Result<(), AgentCliError> {
        self.http
            .get(format!("{}/v1/health", self.base_url))
            .send()
            .await?
            .error_for_status()?;

        Ok(())
    }

    async fn get_session(&self, id: Uuid) -> Result<ChatSession, AgentCliError> {
        let session = self
            .http
            .get(format!("{}/v1/sessions/{}", self.base_url, id))
            .send()
            .await?
            .error_for_status()?
            .json::<ChatSession>()
            .await?;

        Ok(session)
    }

    async fn create_session(&self) -> Result<ChatSession, AgentCliError> {
        let session = self
            .http
            .post(format!("{}/v1/sessions", self.base_url))
            .send()
            .await?
            .error_for_status()?
            .json::<ChatSession>()
            .await?;

        Ok(session)
    }

    async fn connect_events(&self) -> Result<reqwest::Response, AgentCliError> {
        let response = self
            .http
            .get(format!("{}/v1/events", self.base_url))
            .send()
            .await?
            .error_for_status()?;

        Ok(response)
    }

    async fn post_message(&self, session_id: Uuid, text: &str) -> Result<(), AgentCliError> {
        self.http
            .post(format!(
                "{}/v1/sessions/{}/messages",
                self.base_url, session_id
            ))
            .json(&json!({
                "user_message": {
                    "role": "user",
                    "content": [
                        {
                            "type": "input_text",
                            "text": text
                        }
                    ]
                }
            }))
            .send()
            .await?
            .error_for_status()?;

        Ok(())
    }
}

pub async fn run(base_url: String, session_id: Option<Uuid>) -> Result<(), AgentCliError> {
    let client = ChatApiClient::new(base_url);

    // check server health
    client.health().await?;

    let mut session = match session_id {
        Some(id) => client.get_session(id).await?,
        None => client.create_session().await?,
    };

    let mut events = client.connect_events().await?;
    let mut event_buffer = String::new();

    println!("commander chat");
    println!("server: {}", client.base_url);
    println!("session: {}", session.id);

    let mut prompt = format!("\n{}\n{}", session.id, PROMPT);

    let mut rl = DefaultEditor::new().map_err(|e| AgentCliError::Readline(e.to_string()))?;

    loop {
        match rl.readline(&prompt) {
            Ok(line) => {
                let line = line.trim();

                if line.is_empty() {
                    continue;
                }

                let _ = rl.add_history_entry(line);

                match line {
                    "/exit" | "/quit" => break,
                    "/new" => {
                        session = client.create_session().await?;
                        prompt = format!("\n{}\n{}", session.id, PROMPT);
                        println!("new session: {}", session.id);
                    }
                    _ if line.starts_with('/') => {
                        println!("unknown command: {line}");
                    }
                    _ => {
                        // Posting a message only starts the agent turn; output arrives later via SSE.
                        client.post_message(session.id, line).await?;

                        // The event stream is shared by all sessions, so keep only this turn's session.
                        let current_session = session.id.to_string();

                        // Network chunks do not necessarily align with SSE event boundaries.
                        'turn: while let Some(chunk) = events.chunk().await? {
                            event_buffer.push_str(&String::from_utf8_lossy(&chunk));

                            // SSE events are separated by a blank line.
                            // event: xxx
                            // data: {"yyy": "zzz", ...}
                            while let Some(index) = event_buffer.find("\n\n") {
                                let raw_event = event_buffer[..index].to_string();
                                event_buffer = event_buffer[index + 2..].to_string();

                                let mut event_name = "";
                                let mut event_data = String::new();

                                for line in raw_event.lines() {
                                    let line = line.trim_end_matches('\r');

                                    if let Some(value) = line.strip_prefix("event:") {
                                        event_name = value.trim();
                                    } else if let Some(value) = line.strip_prefix("data:") {
                                        event_data.push_str(value.trim());
                                    }
                                }

                                if event_name.is_empty() || event_data.is_empty() {
                                    continue;
                                }

                                // Ignore malformed events and keep-alives that do not contain JSON data.
                                let Ok(data) = serde_json::from_str::<Value>(&event_data) else {
                                    continue;
                                };

                                // Ignore events for other sessions on the shared event stream.
                                if data.get("session_id").and_then(|v| v.as_str())
                                    != Some(current_session.as_str())
                                {
                                    continue;
                                }

                                match event_name {
                                    "assistant_message_created" => {
                                        if let Some(content) =
                                            data.get("content").and_then(|v| v.as_str())
                                        {
                                            println!("{content}");
                                        }
                                    }
                                    "tool_call_started" => {
                                        let tool_name = data
                                            .get("tool_name")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("tool");
                                        println!("[tool call] {tool_name}");

                                        if let Some(arguments) = data.get("arguments") {
                                            let pretty = serde_json::to_string_pretty(arguments)
                                                .unwrap_or_else(|_| arguments.to_string());

                                            let arguments = if pretty.chars().count() > MAX_CHARS {
                                                let truncated = pretty
                                                    .chars()
                                                    .take(MAX_CHARS)
                                                    .collect::<String>();

                                                format!("{truncated}\n... (truncated)")
                                            } else {
                                                pretty
                                            };

                                            println!("[tool call]\n{arguments}");
                                        }
                                    }
                                    "tool_call_finished" => {
                                        let tool_name = data
                                            .get("tool_name")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("tool");
                                        let status = data
                                            .get("status")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("unknown");
                                        println!("[tool call] {tool_name}: {status}");

                                        if let Some(output) = data.get("output") {
                                            let pretty = serde_json::to_string_pretty(output)
                                                .unwrap_or_else(|_| output.to_string());

                                            let output = if pretty.chars().count() > MAX_CHARS {
                                                let truncated = pretty
                                                    .chars()
                                                    .take(MAX_CHARS)
                                                    .collect::<String>();

                                                format!("{truncated}\n... (truncated)")
                                            } else {
                                                pretty
                                            };

                                            println!("[tool call output]\n{output}");
                                        }
                                    }
                                    "tool_call_approval_requested" => {
                                        let tool_name = data
                                            .get("tool_name")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("tool");
                                        println!("[approval requested] {tool_name}");
                                        println!("Run /approve or /deny.");
                                        break 'turn;
                                    }
                                    "agent_turn_completed" => {
                                        // Stop waiting once this turn completes.
                                        break 'turn;
                                    }
                                    "agent_turn_failed" => {
                                        let message = data
                                            .get("message")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("agent turn failed");
                                        println!("[error] {message}");
                                        // Return to the prompt after showing the failure.
                                        break 'turn;
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                // Ctrl+C
                println!("^C");
                break;
            }
            Err(ReadlineError::Eof) => {
                // Ctrl+D
                break;
            }
            Err(e) => {
                return Err(AgentCliError::Readline(e.to_string()));
            }
        }
    }

    Ok(())
}
