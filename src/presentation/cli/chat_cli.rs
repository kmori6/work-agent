use crate::application::usecase::agent_usecase::Attachment;
use crate::domain::model::chat_session::ChatSession;
use crate::domain::model::message::MessageContent;
use crate::presentation::error::agent_cli_error::AgentCliError;
use crate::presentation::util::attachment::load_attachment;
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use serde_json::{Value, json};
use std::path::PathBuf;
use uuid::Uuid;

const PROMPT: &str = "\x1b[38;2;0;71;171m❯\x1b[0m ";
const FILE_ICON: &str = "@";
const MAX_CHARS: usize = 200;

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

    async fn post_message(
        &self,
        session_id: Uuid,
        text: &str,
        attached_files: &[PathBuf],
    ) -> Result<(), AgentCliError> {
        let mut content = Vec::<Value>::new();

        for path in attached_files {
            let attachment = load_attachment(path).map_err(|err| {
                AgentCliError::Io(std::io::Error::other(format!(
                    "failed to load attachment {}: {err}",
                    path.display()
                )))
            })?;

            let value = match attachment {
                Attachment::Image(image) => serde_json::to_value(MessageContent::InputImage(image)),
                Attachment::File(file) => serde_json::to_value(MessageContent::InputFile(file)),
            }
            .map_err(|err| {
                AgentCliError::Io(std::io::Error::other(format!(
                    "failed to encode attachment {}: {err}",
                    path.display()
                )))
            })?;

            content.push(value);
        }

        content.insert(
            0,
            json!({
                "type": "input_text",
                "text": text
            }),
        );

        self.http
            .post(format!(
                "{}/v1/sessions/{}/messages",
                self.base_url, session_id
            ))
            .json(&json!({
                "user_message": {
                    "role": "user",
                    "content": content
                }
            }))
            .send()
            .await?
            .error_for_status()?;

        Ok(())
    }

    async fn resolve_approval(
        &self,
        session_id: Uuid,
        decision: &str,
    ) -> Result<(), AgentCliError> {
        self.http
            .post(format!("{}/v1/approvals/{}", self.base_url, session_id))
            .json(&json!({
                "decision": decision
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

    let mut attached_files = Vec::<PathBuf>::new();
    let mut prompt = format!(
        "\n\x1b[90m{} | files {}\x1b[0m\n{}",
        session.id,
        attached_files.len(),
        PROMPT
    );

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
                    "/exit" => break,
                    "/new" => {
                        session = client.create_session().await?;
                        attached_files.clear();
                        prompt = format!(
                            "\n\x1b[90m{} | files {}\x1b[0m\n{}",
                            session.id,
                            attached_files.len(),
                            PROMPT
                        );
                        println!("new session: {}", session.id);
                    }
                    "/attach" => {
                        println!("usage: /attach <files...>");
                    }
                    _ if line.starts_with("/attach ") => {
                        let paths = line
                            .split_whitespace()
                            .skip(1)
                            .map(|path| path.trim_matches('\'').trim_matches('"'))
                            .map(PathBuf::from)
                            .collect::<Vec<_>>();

                        let mut attached = Vec::new();
                        for path in paths {
                            match std::fs::metadata(&path) {
                                Ok(metadata) if metadata.is_file() => {
                                    let bytes = metadata.len();
                                    let size = if bytes < 1024 {
                                        format!("{bytes} B")
                                    } else if bytes < 1024 * 1024 {
                                        format!("{:.1} KB", bytes as f64 / 1024.0)
                                    } else {
                                        format!("{:.1} MB", bytes as f64 / 1024.0 / 1024.0)
                                    };

                                    attached_files.push(path.clone());
                                    attached.push((attached_files.len(), path, size));
                                }
                                Ok(_) => {
                                    println!("not a file: {}", path.display());
                                }
                                Err(err) => {
                                    println!("failed to attach {}: {err}", path.display());
                                }
                            }
                        }

                        if !attached.is_empty() {
                            prompt = format!(
                                "\n\x1b[90m{} | files {}\x1b[0m\n{}",
                                session.id,
                                attached_files.len(),
                                PROMPT
                            );

                            println!("attached");

                            for (index, path, size) in attached {
                                println!("  {FILE_ICON} {index}  {}  {size}", path.display());
                            }
                        }
                    }
                    "/files" => {
                        if attached_files.is_empty() {
                            println!("no attached files");
                        } else {
                            println!("attached files");

                            for (index, path) in attached_files.iter().enumerate() {
                                let size = match std::fs::metadata(path) {
                                    Ok(metadata) => {
                                        let bytes = metadata.len();

                                        if bytes < 1024 {
                                            format!("{bytes} B")
                                        } else if bytes < 1024 * 1024 {
                                            format!("{:.1} KB", bytes as f64 / 1024.0)
                                        } else {
                                            format!("{:.1} MB", bytes as f64 / 1024.0 / 1024.0)
                                        }
                                    }
                                    Err(_) => "missing".to_string(),
                                };

                                println!("  {FILE_ICON} {}  {}  {size}", index + 1, path.display());
                            }
                        }
                    }
                    "/detach" => {
                        println!("usage: /detach <index|all>");
                    }
                    _ if line.starts_with("/detach ") => {
                        let target = line.trim_start_matches("/detach ").trim();

                        if target == "all" {
                            attached_files.clear();

                            prompt = format!(
                                "\n\x1b[90m{} | files {}\x1b[0m\n{}",
                                session.id,
                                attached_files.len(),
                                PROMPT
                            );

                            println!("detached all files");
                        } else {
                            let Ok(index) = target.parse::<usize>() else {
                                println!("invalid file index: {target}");
                                continue;
                            };

                            if index == 0 || index > attached_files.len() {
                                println!("file index out of range: {index}");
                                continue;
                            }

                            let detached = attached_files.remove(index - 1);

                            prompt = format!(
                                "\n\x1b[90m{} | files {}\x1b[0m\n{}",
                                session.id,
                                attached_files.len(),
                                PROMPT
                            );

                            println!("detached");
                            println!("  {FILE_ICON} {index}  {}", detached.display());
                        }
                    }
                    "/approve" => {
                        client.resolve_approval(session.id, "approved").await?;
                        wait_events(&mut events, &mut event_buffer, session.id).await?;
                    }
                    "/deny" => {
                        client.resolve_approval(session.id, "denied").await?;
                        wait_events(&mut events, &mut event_buffer, session.id).await?;
                    }
                    _ if line.starts_with('/') => {
                        println!("unknown command: {line}");
                    }
                    _ => {
                        // Posting a message only starts the agent turn; output arrives later via SSE.
                        client
                            .post_message(session.id, line, &attached_files)
                            .await?;

                        attached_files.clear();

                        // Reconstruct the prompt to show the files still attached for the next message.
                        prompt = format!(
                            "\n\x1b[90m{} | files {}\x1b[0m\n{}",
                            session.id,
                            attached_files.len(),
                            PROMPT
                        );

                        // The event stream is shared by all sessions, so keep only this turn's session.
                        wait_events(&mut events, &mut event_buffer, session.id).await?;
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

async fn wait_events(
    events: &mut reqwest::Response,
    event_buffer: &mut String,
    session_id: Uuid,
) -> Result<(), AgentCliError> {
    let current_session = session_id.to_string();

    'turn: while let Some(chunk) = events.chunk().await? {
        event_buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(index) = event_buffer.find("\n\n") {
            let raw_event = event_buffer[..index].to_string();
            *event_buffer = event_buffer[index + 2..].to_string();

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

            let Ok(data) = serde_json::from_str::<Value>(&event_data) else {
                continue;
            };

            if data.get("session_id").and_then(|v| v.as_str()) != Some(current_session.as_str()) {
                continue;
            }

            match event_name {
                "assistant_message_created" => {
                    if let Some(content) = data.get("content").and_then(|v| v.as_str()) {
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
                            let truncated = pretty.chars().take(MAX_CHARS).collect::<String>();

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
                            let truncated = pretty.chars().take(MAX_CHARS).collect::<String>();

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

    Ok(())
}
