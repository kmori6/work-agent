use crate::presentation::error::agent_cli_error::AgentCliError;
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;

pub async fn run() -> Result<(), AgentCliError> {
    println!("Welcome to Commander Chat CLI!");

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
