use crate::domain::model::tool_execution_rule::ToolExecutionRuleAction;

#[derive(Debug)]
pub enum AgentCommand {
    Help,
    NewSession,
    Sessions,
    SwitchSession(String),
    Approve,
    Deny,
    ToolRules,
    SetToolRule {
        tool_name: String,
        action: ToolExecutionRuleAction,
    },
    Attach(Vec<String>),
    Detach(Vec<String>),
    Staged,
    Exit,
    Unknown(String),
    Invalid(String),
    UserMessage(String),
}

pub fn parse_command(line: &str) -> Option<AgentCommand> {
    let input = line.trim();

    if input.is_empty() {
        return None;
    }

    Some(match input {
        "/help" => AgentCommand::Help,
        "/new" => AgentCommand::NewSession,
        "/sessions" => AgentCommand::Sessions,
        "/approve" => AgentCommand::Approve,
        "/deny" => AgentCommand::Deny,
        "/tool-rules" => AgentCommand::ToolRules,
        "/tool-rule" => {
            AgentCommand::Invalid("usage: /tool-rule <tool_name> <allow|ask|deny>".to_string())
        }
        "/attach" => AgentCommand::Attach(Vec::new()),
        "/detach" => AgentCommand::Detach(Vec::new()),
        "/attachments" | "/staged" => AgentCommand::Staged,
        "/exit" | "/quit" => AgentCommand::Exit,
        _ if input.starts_with("/session ") => {
            AgentCommand::SwitchSession(input.trim_start_matches("/session ").trim().to_string())
        }
        _ if input.starts_with("/attach ") => AgentCommand::Attach(command_args(input)),
        _ if input.starts_with("/detach ") => AgentCommand::Detach(command_args(input)),
        _ if input.starts_with("/tool-rule ") => parse_tool_rule_command(input),
        _ if input.starts_with('/') => AgentCommand::Unknown(input.to_string()),
        _ => AgentCommand::UserMessage(input.to_string()),
    })
}

fn command_args(input: &str) -> Vec<String> {
    shell_words(input).into_iter().skip(1).collect()
}

pub fn shell_words(input: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut escaped = false;

    for ch in input.chars() {
        if escaped {
            word.push(ch);
            escaped = false;
            continue;
        }

        if ch == '\\' {
            escaped = true;
            continue;
        }

        if let Some(quote_char) = quote {
            if ch == quote_char {
                quote = None;
            } else {
                word.push(ch);
            }
            continue;
        }

        match ch {
            '"' | '\'' => quote = Some(ch),
            ch if ch.is_whitespace() => {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            }
            _ => word.push(ch),
        }
    }

    if escaped {
        word.push('\\');
    }

    if !word.is_empty() {
        words.push(word);
    }

    words
}

fn parse_tool_rule_command(input: &str) -> AgentCommand {
    let parts: Vec<&str> = input.split_whitespace().collect();

    if parts.len() != 3 {
        return AgentCommand::Invalid("usage: /tool-rule <tool_name> <allow|ask|deny>".to_string());
    }

    let Some(action) = ToolExecutionRuleAction::from_str(parts[2]) else {
        return AgentCommand::Invalid("usage: /tool-rule <tool_name> <allow|ask|deny>".to_string());
    };

    AgentCommand::SetToolRule {
        tool_name: parts[1].to_string(),
        action,
    }
}
