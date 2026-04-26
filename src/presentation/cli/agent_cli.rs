use std::borrow::Cow;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;

use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use reedline::{Prompt, PromptEditMode, PromptHistorySearch, Reedline, Signal};
use termimad::print_text;
use uuid::Uuid;

use crate::application::usecase::agent_usecase::{
    AgentEvent, AgentUsecase, HandleAgentInput, HandleAgentOutput,
};
use crate::application::usecase::tool_execution_rule_usecase::ToolExecutionRuleUsecase;
use crate::domain::model::attachment::Attachment;
use crate::domain::port::llm_provider::LlmProvider;
use crate::domain::repository::chat_message_repository::ChatMessageRepository;
use crate::domain::repository::chat_session_repository::ChatSessionRepository;
use crate::domain::repository::token_usage_repository::TokenUsageRepository;
use crate::domain::repository::tool_approval_repository::ToolApprovalRepository;
use crate::domain::repository::tool_execution_rule_repository::ToolExecutionRuleRepository;
use crate::domain::service::agent_service::AgentEvent as AgentProgressEvent;
use crate::presentation::command::agent_command::{AgentCommand, parse_command, shell_words};
use crate::presentation::error::agent_cli_error::AgentCliError;
use crate::presentation::util::attachment::load_attachment;

const SESSION_LIST_LIMIT: usize = 10;
const MAX_ARGUMENT_PREVIEW_CHARS: usize = 800;
const PROMPT_ARROW: &str = "\x1b[38;2;0;71;171m❯\x1b[0m";
const ASSISTANT_LABEL: &str = "\x1b[38;2;0;71;171mCommander\x1b[0m";

struct ReplState {
    session_id: Uuid,
    staged_files: Vec<PathBuf>,
}

impl ReplState {
    fn new(session_id: Uuid) -> Self {
        Self {
            session_id,
            staged_files: Vec::new(),
        }
    }
}

pub async fn run<L, S, M, T, A, TR>(
    usecase: &AgentUsecase<L, S, M, T, A>,
    tool_execution_rule_usecase: &ToolExecutionRuleUsecase<TR>,
) -> Result<(), AgentCliError>
where
    L: LlmProvider,
    S: ChatSessionRepository,
    M: ChatMessageRepository,
    T: TokenUsageRepository,
    A: ToolApprovalRepository,
    TR: ToolExecutionRuleRepository,
{
    let session = usecase.start_session().await?;
    let mut state = ReplState::new(session.id);

    println!("commander agent");
    println!("session: {}", session.id);
    println!("type /help for commands, /exit to quit");

    repl_loop(&mut state, usecase, tool_execution_rule_usecase).await
}

async fn repl_loop<L, S, M, T, A, TR>(
    state: &mut ReplState,
    usecase: &AgentUsecase<L, S, M, T, A>,
    tool_execution_rule_usecase: &ToolExecutionRuleUsecase<TR>,
) -> Result<(), AgentCliError>
where
    L: LlmProvider,
    S: ChatSessionRepository,
    M: ChatMessageRepository,
    T: TokenUsageRepository,
    A: ToolApprovalRepository,
    TR: ToolExecutionRuleRepository,
{
    let mut line_editor = Reedline::create().with_ansi_colors(true);

    loop {
        let Some(line) = read_repl_line(&mut line_editor, state)? else {
            println!();
            break;
        };

        let Some(command) = dropped_file_paths(&line)
            .map(AgentCommand::Attach)
            .or_else(|| parse_command(&line))
        else {
            continue;
        };

        println!();
        if handle_command(command, state, usecase, tool_execution_rule_usecase).await? {
            break;
        }
        println!();
    }

    Ok(())
}

fn read_repl_line(
    line_editor: &mut Reedline,
    state: &ReplState,
) -> Result<Option<String>, AgentCliError> {
    let prompt = ReplPrompt::new(state.staged_files.len());
    let signal = line_editor
        .read_line(&prompt)
        .map_err(|err| AgentCliError::Readline(err.to_string()))?;

    match signal {
        Signal::Success(line) | Signal::ExternalBreak(line) => Ok(Some(line)),
        Signal::CtrlC => {
            println!("^C");
            Ok(Some(String::new()))
        }
        Signal::CtrlD => Ok(None),
        other => Err(AgentCliError::Readline(format!(
            "unsupported reedline signal: {other:?}"
        ))),
    }
}

async fn handle_command<L, S, M, T, A, TR>(
    command: AgentCommand,
    state: &mut ReplState,
    usecase: &AgentUsecase<L, S, M, T, A>,
    tool_execution_rule_usecase: &ToolExecutionRuleUsecase<TR>,
) -> Result<bool, AgentCliError>
where
    L: LlmProvider,
    S: ChatSessionRepository,
    M: ChatMessageRepository,
    T: TokenUsageRepository,
    A: ToolApprovalRepository,
    TR: ToolExecutionRuleRepository,
{
    match command {
        AgentCommand::Exit => return Ok(true),

        AgentCommand::Help => {
            println!(
                "{}",
                [
                    "/new              start a new session",
                    "/sessions         list recent sessions",
                    "/session <id>     switch to a session",
                    "/approve          approve pending tool execution",
                    "/deny             deny pending tool execution",
                    "/tools            list tools with approval rules",
                    "/tool <tool> <allow|ask|deny>  set tool rule",
                    "/attach <files>   attach files",
                    "/detach <files|indexes>  detach files",
                    "/attachments      show attachments",
                    "/exit             quit",
                ]
                .join("\n")
            );
        }

        AgentCommand::NewSession => {
            let session = usecase.start_session().await?;
            state.session_id = session.id;
            state.staged_files.clear();
            println!("new session: {}", session.id);
        }

        AgentCommand::Sessions => {
            let sessions = usecase.list_sessions(SESSION_LIST_LIMIT).await?;
            let content = if sessions.is_empty() {
                "no sessions".to_string()
            } else {
                sessions
                    .iter()
                    .map(|s| {
                        let marker = if s.id == state.session_id { "*" } else { " " };
                        format!("{marker} {}  updated={}", s.id, s.updated_at)
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            println!("{content}");
        }

        AgentCommand::SwitchSession(raw_id) => {
            let Ok(session_id) = Uuid::parse_str(&raw_id) else {
                println!("invalid session id: {raw_id}");
                return Ok(false);
            };
            match usecase.find_session(session_id).await? {
                Some(session) => {
                    state.session_id = session.id;
                    println!("switched to: {}", session.id);
                }
                None => {
                    println!("session not found: {session_id}");
                }
            }
        }

        AgentCommand::Staged => {
            println!("{}", format_attachments(&state.staged_files));
        }

        AgentCommand::Attach(paths) => {
            if paths.is_empty() {
                println!("usage: /attach <files...>");
            } else {
                attach_paths(state, paths);
            }
        }

        AgentCommand::Detach(paths) => {
            if state.staged_files.is_empty() {
                println!("{}", format_attachments(&state.staged_files));
            } else if !paths.is_empty() {
                detach_paths(state, &paths);
            } else {
                println!("usage: /detach <files...>");
                println!("{}", format_attachments(&state.staged_files));
            }
        }

        AgentCommand::Approve => {
            let session_id = state.session_id;
            let (output, printed_progress_events) =
                run_with_progress(|tx| usecase.approve_approval(session_id, tx)).await?;
            apply_output(output, printed_progress_events);
        }

        AgentCommand::Deny => {
            let session_id = state.session_id;
            let (output, printed_progress_events) =
                run_with_progress(|tx| usecase.deny_approval(session_id, tx)).await?;
            apply_output(output, printed_progress_events);
        }

        AgentCommand::Tools => {
            let summaries = usecase.tool_rule_summaries().await?;

            let mut lines = vec![format!(
                "  {:<20} {:<6} {:<6} {:<6} {}",
                "tool", "action", "policy", "rule", "source"
            )];

            lines.extend(summaries.iter().map(|summary| {
                format!(
                    "  {:<20} {:<6} {:<6} {:<6} {}",
                    summary.tool_name,
                    summary.action.as_str(),
                    summary.policy.as_str(),
                    summary.rule.map_or("-", |rule| rule.as_str()),
                    summary.source.as_str(),
                )
            }));

            println!("{}", lines.join("\n"));
        }

        AgentCommand::SetToolRule { tool_name, action } => {
            let action_text = action.as_str().to_string();
            tool_execution_rule_usecase
                .set(tool_name.clone(), action)
                .await?;
            println!("tool rule saved: {tool_name} -> {action_text}");
        }

        AgentCommand::Invalid(msg) => {
            println!("{msg}");
        }

        AgentCommand::Unknown(name) => {
            println!("unknown command: {name}");
        }

        AgentCommand::UserMessage(text) => {
            // Collect and clear staged files
            let staged = std::mem::take(&mut state.staged_files);
            if !staged.is_empty() {
                println!("using attachments\n{}", format_indexed_paths(&staged));
                println!();
            }
            let attachments: Vec<Attachment> = staged
                .iter()
                .filter_map(|p| load_attachment(p).ok())
                .collect();

            let input = HandleAgentInput {
                session_id: state.session_id,
                user_input: text,
                attachments,
            };
            let (output, printed_progress_events) =
                run_with_progress(|tx| usecase.handle(input, tx)).await?;
            apply_output(output, printed_progress_events);
        }
    }

    Ok(false)
}

const ATTACHMENT_ICON: &str = "@";

struct ReplPrompt {
    text: String,
}

impl ReplPrompt {
    fn new(attachment_count: usize) -> Self {
        let text = if attachment_count == 0 {
            format!("{PROMPT_ARROW} ")
        } else {
            format!("{ATTACHMENT_ICON}{attachment_count} {PROMPT_ARROW} ")
        };

        Self { text }
    }
}

impl Prompt for ReplPrompt {
    fn render_prompt_left(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.text)
    }

    fn render_prompt_right(&self) -> Cow<'_, str> {
        Cow::Borrowed("")
    }

    fn render_prompt_indicator(&self, _prompt_mode: PromptEditMode) -> Cow<'_, str> {
        Cow::Borrowed("")
    }

    fn render_prompt_multiline_indicator(&self) -> Cow<'_, str> {
        Cow::Borrowed("... ")
    }

    fn render_prompt_history_search_indicator(
        &self,
        history_search: PromptHistorySearch,
    ) -> Cow<'_, str> {
        format!("(history: {}) ", history_search.term).into()
    }
}

fn attach_paths<I, S>(state: &mut ReplState, paths: I)
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut attached = Vec::new();
    let mut missing = Vec::new();

    for raw in paths {
        let raw = raw.as_ref().trim();
        if raw.is_empty() {
            continue;
        }

        let path = path_from_input_token(raw);
        if path.exists() && path.is_file() {
            let index = state.staged_files.len() + 1;
            state.staged_files.push(path.clone());
            attached.push((index, path));
        } else {
            missing.push(raw.to_string());
        }
    }

    let mut sections = Vec::new();
    if !attached.is_empty() {
        sections.push(format!(
            "attached\n{}",
            format_indexed_path_entries(&attached)
        ));
    }
    if !missing.is_empty() {
        sections.push(format!("not found\n{}", format_raw_file_lines(&missing)));
    }

    if sections.is_empty() {
        sections.push("no files".to_string());
    }

    println!("{}", sections.join("\n\n"));
}

fn dropped_file_paths(input: &str) -> Option<Vec<String>> {
    let paths: Vec<PathBuf> = shell_words(input)
        .into_iter()
        .map(|word| path_from_input_token(&word))
        .collect();

    if paths.is_empty() || !paths.iter().all(|path| path.exists() && path.is_file()) {
        return None;
    }

    Some(paths.iter().map(|path| display_path(path)).collect())
}

fn path_from_input_token(token: &str) -> PathBuf {
    let token = token.trim();
    let path = token
        .strip_prefix("file://")
        .map(|path| percent_decode(path.strip_prefix("localhost").unwrap_or(path)))
        .unwrap_or_else(|| token.to_string());

    PathBuf::from(path)
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = &input[index + 1..index + 3];
            if let Ok(value) = u8::from_str_radix(hex, 16) {
                output.push(value);
                index += 3;
                continue;
            }
        }

        output.push(bytes[index]);
        index += 1;
    }

    String::from_utf8_lossy(&output).into_owned()
}

fn detach_paths(state: &mut ReplState, paths: &[String]) {
    let mut detached = Vec::new();
    let mut matched = vec![false; paths.len()];
    let staged_files = std::mem::take(&mut state.staged_files);

    for (index, path) in staged_files.into_iter().enumerate() {
        let attachment_index = index + 1;
        let Some(index) = paths
            .iter()
            .position(|requested| path_matches_request(attachment_index, &path, requested))
        else {
            state.staged_files.push(path);
            continue;
        };

        matched[index] = true;
        detached.push((attachment_index, path));
    }

    let missing: Vec<String> = paths
        .iter()
        .enumerate()
        .filter(|(index, _)| !matched[*index])
        .map(|(_, path)| path.clone())
        .collect();

    let mut sections = Vec::new();
    if !detached.is_empty() {
        sections.push(format!(
            "detached\n{}",
            format_indexed_path_entries(&detached)
        ));
        sections.push(format_attachments(&state.staged_files));
    }
    if !missing.is_empty() {
        sections.push(format!("not attached\n{}", format_raw_file_lines(&missing)));
    }
    if sections.is_empty() {
        sections.push(format_attachments(&state.staged_files));
    }

    println!("{}", sections.join("\n\n"));
}

fn path_matches_request(index: usize, path: &Path, requested: &str) -> bool {
    requested
        .parse::<usize>()
        .is_ok_and(|requested| requested == index)
        || display_path(path) == requested
        || path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == requested)
}

fn format_attachments(paths: &[PathBuf]) -> String {
    if paths.is_empty() {
        return "attachments: none".to_string();
    }

    let files = paths
        .iter()
        .enumerate()
        .map(|(index, path)| {
            format!(
                "  [{}] {} {}\n      {}",
                index + 1,
                ATTACHMENT_ICON,
                file_name(path),
                display_path(path)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!("attachments\n{files}")
}

fn format_indexed_paths(paths: &[PathBuf]) -> String {
    format_indexed_path_iter(
        paths
            .iter()
            .enumerate()
            .map(|(index, path)| (index + 1, path)),
    )
}

fn format_indexed_path_entries(paths: &[(usize, PathBuf)]) -> String {
    format_indexed_path_iter(paths.iter().map(|(index, path)| (*index, path)))
}

fn format_indexed_path_iter<'a>(paths: impl Iterator<Item = (usize, &'a PathBuf)>) -> String {
    paths
        .map(|(index, path)| format!("  [{}] {} {}", index, ATTACHMENT_ICON, file_name(path)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_raw_file_lines(paths: &[String]) -> String {
    paths
        .iter()
        .map(|path| format!("  {} {}", ATTACHMENT_ICON, path))
        .collect::<Vec<_>>()
        .join("\n")
}

fn display_path(path: &Path) -> String {
    path.display().to_string()
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(ToString::to_string)
        .unwrap_or_else(|| display_path(path))
}

async fn run_with_progress<F, Fut, E>(
    make_future: F,
) -> Result<(HandleAgentOutput, bool), AgentCliError>
where
    F: FnOnce(tokio::sync::mpsc::Sender<AgentProgressEvent>) -> Fut,
    Fut: Future<Output = Result<HandleAgentOutput, E>>,
    AgentCliError: From<E>,
{
    let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentProgressEvent>(32);
    let fut = make_future(tx);
    tokio::pin!(fut);

    let mut reporter = ProgressReporter::default();

    let output = loop {
        tokio::select! {
            result = &mut fut => {
                while let Ok(event) = rx.try_recv() {
                    handle_progress_event(event, &mut reporter);
                }
                reporter.stop_thinking();
                let output = result.map_err(AgentCliError::from)?;
                break (output, reporter.printed_events);
            }
            Some(event) = rx.recv() => {
                handle_progress_event(event, &mut reporter);
            }
        }
    };

    Ok(output)
}

fn handle_progress_event(event: AgentProgressEvent, reporter: &mut ProgressReporter) {
    match event {
        AgentProgressEvent::LlmStarted => {
            reporter.start_thinking();
        }
        AgentProgressEvent::LlmFinished => {
            reporter.stop_thinking();
        }
        AgentProgressEvent::ToolStarted { tool_name, call_id } => {
            reporter.println(format!("[tool call] {tool_name} ({call_id})"));
        }
        AgentProgressEvent::ToolFinished {
            tool_name,
            call_id,
            success,
        } => {
            reporter.println(format!(
                "[tool result] {tool_name} ({call_id}): {}",
                if success { "success" } else { "failed" }
            ));
        }
    }
}

fn apply_output(output: HandleAgentOutput, mut separate_from_progress: bool) {
    for event in &output.events {
        match event {
            AgentEvent::AssistantMessage(msg) => {
                if !msg.is_empty() {
                    if separate_from_progress {
                        println!();
                        separate_from_progress = false;
                    }
                    println!("{ASSISTANT_LABEL}");
                    print_text(msg);
                }
            }
            AgentEvent::ToolConfirmationRequested {
                tool_name,
                arguments,
                ..
            } => {
                let pretty = serde_json::to_string_pretty(arguments)
                    .unwrap_or_else(|_| arguments.to_string());
                let preview = truncate(pretty, MAX_ARGUMENT_PREVIEW_CHARS);
                if separate_from_progress {
                    println!();
                    separate_from_progress = false;
                }
                println!(
                    "[confirmation requested] {tool_name}\n{preview}\nRun /approve to execute, or /deny to cancel."
                );
            }
        }
    }

    println!();
    println!(
        "tokens: in={} out={} | context={}/{} ({}%)",
        output.usage.input_tokens,
        output.usage.output_tokens,
        output.context_input_tokens,
        output.context_window_tokens,
        output.context_percent_used,
    );
}

fn truncate(text: String, max: usize) -> String {
    if text.chars().count() <= max {
        return text;
    }
    let t: String = text.chars().take(max).collect();
    format!("{t}\n... (truncated)")
}

#[derive(Default)]
struct ProgressReporter {
    spinner: Option<ProgressBar>,
    printed_events: bool,
}

impl ProgressReporter {
    fn start_thinking(&mut self) {
        if self.spinner.is_some() {
            return;
        }

        let spinner = ProgressBar::with_draw_target(None, ProgressDrawTarget::stdout());
        spinner.set_style(
            ProgressStyle::with_template("{spinner} {msg}")
                .expect("spinner template should be valid")
                .tick_strings(&["-", "\\", "|", "/"]),
        );
        spinner.set_message("LLM is thinking ...");
        spinner.enable_steady_tick(Duration::from_millis(120));

        self.spinner = Some(spinner);
    }

    fn stop_thinking(&mut self) {
        if let Some(spinner) = self.spinner.take() {
            spinner.finish_and_clear();
        }
    }

    fn println(&mut self, line: impl AsRef<str>) {
        self.stop_thinking();
        println!("{}", line.as_ref());
        self.printed_events = true;
    }
}
