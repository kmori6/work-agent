use crate::domain::model::chat_session::ChatSession;
use crate::presentation::error::agent_cli_error::AgentCliError;
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use uuid::Uuid;

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
}

pub async fn run(base_url: String, session_id: Option<Uuid>) -> Result<(), AgentCliError> {
    let client = ChatApiClient::new(base_url);

    // check server health
    client.health().await?;

    let session = match session_id {
        Some(id) => client.get_session(id).await?,
        None => client.create_session().await?,
    };

    println!("commander chat");
    println!("server: {}", client.base_url);
    println!("session: {}", session.id);

    let mut rl = DefaultEditor::new().map_err(|e| AgentCliError::Readline(e.to_string()))?;

    loop {
        match rl.readline("> ") {
            Ok(line) => {
                let _ = rl.add_history_entry(&line);
                // TODO: Process the input line and generate a response
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
