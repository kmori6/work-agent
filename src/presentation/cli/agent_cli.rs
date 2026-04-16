use crate::application::usecase::agent_usecase::{
    AgentEvent, AgentUsecase, HandleAgentInput, HandleAgentOutput,
};
use crate::domain::port::llm_provider::LlmProvider;
use crate::domain::service::agent_service::AgentProgressEvent;
use crate::presentation::error::agent_cli_error::AgentCliError;
use reedline::{
    MouseClickMode, Prompt, PromptEditMode, PromptHistorySearch, PromptHistorySearchStatus,
    Reedline, Signal,
};
use serde_json::Value;
use std::borrow::Cow;
use std::io::{Write, stderr};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{Duration, MissedTickBehavior, interval};

const SPINNER_TICK_MS: u64 = 120;
const MAX_ARGUMENT_PREVIEW_CHARS: usize = 800;
const SPINNER_FRAMES: [&str; 4] = ["|", "/", "-", "\\"];

pub async fn run<L: LlmProvider>(usecase: &AgentUsecase<L>) -> Result<(), AgentCliError> {
    println!("Agent mock CLI");
    println!("type /help for commands");

    let mut line_editor = build_line_editor();
    let prompt = AgentPrompt;

    loop {
        let Some(line) = read_command(&mut line_editor, &prompt)? else {
            println!();
            break;
        };

        let Some(command) = parse_command(line) else {
            continue;
        };

        match command {
            CliCommand::Help => print_help(),
            CliCommand::Reset => println!("mock session reset"),
            CliCommand::Exit => break,
            CliCommand::Unknown(name) => println!("unknown command: {name}"),
            CliCommand::UserMessage(message) => {
                handle_user_message(usecase, message).await?;
            }
        }
    }

    Ok(())
}

// This prompt keeps the terminal UI simple and predictable.
// We intentionally use ASCII markers so the prompt is easy to read
// even when the terminal mixes English and Japanese input.
struct AgentPrompt;

impl Prompt for AgentPrompt {
    fn render_prompt_left(&self) -> Cow<'_, str> {
        Cow::Borrowed("agent")
    }

    fn render_prompt_right(&self) -> Cow<'_, str> {
        Cow::Borrowed("")
    }

    fn render_prompt_indicator(&self, _prompt_mode: PromptEditMode) -> Cow<'_, str> {
        Cow::Borrowed(" > ")
    }

    fn render_prompt_multiline_indicator(&self) -> Cow<'_, str> {
        Cow::Borrowed("... ")
    }

    fn render_prompt_history_search_indicator(
        &self,
        history_search: PromptHistorySearch,
    ) -> Cow<'_, str> {
        let status = match history_search.status {
            PromptHistorySearchStatus::Passing => "history",
            PromptHistorySearchStatus::Failing => "history-failed",
        };

        format!("({status}: {}) ", history_search.term).into()
    }
}

#[derive(Debug)]
enum CliCommand {
    Help,
    Reset,
    Exit,
    Unknown(String),
    UserMessage(String),
}

#[derive(Debug)]
enum CliStatusEvent {
    ThinkingStarted,
    ThinkingFinished,
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
    Shutdown,
}

impl From<AgentProgressEvent> for CliStatusEvent {
    fn from(value: AgentProgressEvent) -> Self {
        match value {
            AgentProgressEvent::LlmThinkingStarted => Self::ThinkingStarted,
            AgentProgressEvent::LlmThinkingFinished => Self::ThinkingFinished,
            AgentProgressEvent::ToolCallRequested {
                call_id,
                tool_name,
                arguments,
            } => Self::ToolCallRequested {
                call_id,
                tool_name,
                arguments,
            },
            AgentProgressEvent::ToolExecutionFinished {
                call_id,
                tool_name,
                success,
            } => Self::ToolExecutionFinished {
                call_id,
                tool_name,
                success,
            },
        }
    }
}

// The renderer owns the temporary "thinking..." UI so the main CLI loop
// can stay focused on commands and use case calls.
struct CliStatusRenderer {
    tx: mpsc::UnboundedSender<CliStatusEvent>,
    task: JoinHandle<()>,
}

impl CliStatusRenderer {
    fn spawn() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(render_status(rx));

        Self { tx, task }
    }

    fn sender(&self) -> mpsc::UnboundedSender<CliStatusEvent> {
        self.tx.clone()
    }

    async fn shutdown(self) {
        let _ = self.tx.send(CliStatusEvent::Shutdown);
        let _ = self.task.await;
    }
}

fn build_line_editor() -> Reedline {
    Reedline::create().with_mouse_click(MouseClickMode::EnabledWithOsc133)
}

fn read_command(
    line_editor: &mut Reedline,
    prompt: &impl Prompt,
) -> Result<Option<String>, AgentCliError> {
    let signal = line_editor
        .read_line(prompt)
        .map_err(|err| AgentCliError::Readline(err.to_string()))?;

    match signal {
        Signal::Success(line) => Ok(Some(line)),
        Signal::CtrlC => {
            println!();
            Ok(Some(String::new()))
        }
        Signal::CtrlD => Ok(None),
        // Treat an external break as a normal line so we do not discard
        // the user's buffer unexpectedly.
        Signal::ExternalBreak(line) => Ok(Some(line)),
        other => Err(AgentCliError::Readline(format!(
            "unsupported reedline signal: {other:?}"
        ))),
    }
}

fn parse_command(line: String) -> Option<CliCommand> {
    let input = line.trim();

    if input.is_empty() {
        return None;
    }

    Some(match input {
        "/help" => CliCommand::Help,
        "/reset" => CliCommand::Reset,
        "/exit" | "/quit" => CliCommand::Exit,
        _ if input.starts_with('/') => CliCommand::Unknown(input.to_string()),
        _ => CliCommand::UserMessage(input.to_string()),
    })
}

async fn handle_user_message<L: LlmProvider>(
    usecase: &AgentUsecase<L>,
    message: String,
) -> Result<(), AgentCliError> {
    let status_renderer = CliStatusRenderer::spawn();
    let progress_tx = status_renderer.sender();

    let result = usecase
        .handle_with_progress(
            HandleAgentInput {
                user_input: message,
            },
            move |event| {
                let _ = progress_tx.send(event.into());
            },
        )
        .await;

    status_renderer.shutdown().await;

    let output = result?;
    print_agent_output(output);

    Ok(())
}

fn print_agent_output(output: HandleAgentOutput) {
    for event in output.reply {
        match event {
            AgentEvent::AssistantMessage(message) => println!("{message}"),
        }
    }
}

async fn render_status(mut rx: mpsc::UnboundedReceiver<CliStatusEvent>) {
    let mut spinner_active = false;
    let mut frame_index = 0;

    let mut ticker = interval(Duration::from_millis(SPINNER_TICK_MS));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = ticker.tick(), if spinner_active => {
                render_spinner_frame(&mut frame_index);
            }
            maybe_event = rx.recv() => {
                let Some(event) = maybe_event else {
                    clear_status_line_silent();
                    break;
                };

                if handle_status_event(event, &mut spinner_active, &mut frame_index) {
                    break;
                }
            }
        }
    }
}

fn handle_status_event(
    event: CliStatusEvent,
    spinner_active: &mut bool,
    frame_index: &mut usize,
) -> bool {
    match event {
        CliStatusEvent::ThinkingStarted => {
            *spinner_active = true;
            *frame_index = 0;
            false
        }
        CliStatusEvent::ThinkingFinished => {
            *spinner_active = false;
            clear_status_line_silent();
            false
        }
        CliStatusEvent::ToolCallRequested {
            call_id,
            tool_name,
            arguments,
        } => {
            *spinner_active = false;
            clear_status_line_silent();
            print_tool_call(&call_id, &tool_name, &arguments);
            false
        }
        CliStatusEvent::ToolExecutionFinished {
            call_id,
            tool_name,
            success,
        } => {
            *spinner_active = false;
            clear_status_line_silent();
            print_tool_result(&call_id, &tool_name, success);
            false
        }
        CliStatusEvent::Shutdown => {
            *spinner_active = false;
            clear_status_line_silent();
            true
        }
    }
}

fn render_spinner_frame(frame_index: &mut usize) {
    eprint!(
        "\rThinking... {}",
        SPINNER_FRAMES[*frame_index % SPINNER_FRAMES.len()]
    );
    let _ = stderr().flush();
    *frame_index += 1;
}

fn print_tool_call(call_id: &str, tool_name: &str, arguments: &Value) {
    eprintln!("[tool call] {tool_name} ({call_id})");
    eprintln!("{}", format_arguments(arguments));
}

fn print_tool_result(call_id: &str, tool_name: &str, success: bool) {
    eprintln!(
        "[tool result] {tool_name} ({call_id}): {}",
        if success { "success" } else { "failed" }
    );
}

fn clear_status_line() -> Result<(), AgentCliError> {
    eprint!("\r\x1b[2K");
    stderr().flush()?;
    Ok(())
}

fn clear_status_line_silent() {
    let _ = clear_status_line();
}

fn format_arguments(arguments: &Value) -> String {
    let pretty = serde_json::to_string_pretty(arguments).unwrap_or_else(|_| arguments.to_string());

    truncate_for_cli(pretty, MAX_ARGUMENT_PREVIEW_CHARS)
}

fn truncate_for_cli(text: String, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text;
    }

    let truncated: String = text.chars().take(max_chars).collect();
    format!("{truncated}\n... (truncated)")
}

fn print_help() {
    println!("/help  show help");
    println!("/reset reset mock session");
    println!("/exit  quit");
}
