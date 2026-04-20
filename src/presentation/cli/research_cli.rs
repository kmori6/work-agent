use crate::application::usecase::research_usecase::{ResearchUsecase, RunResearchInput};
use crate::domain::port::llm_provider::LlmProvider;
use crate::domain::port::search_provider::SearchProvider;
use crate::presentation::error::agent_cli_error::AgentCliError;
use chrono::Local;
use std::fs;
use std::io::{Write, stdin, stdout};
use std::path::PathBuf;

const RESEARCH_OUTPUT_DIR: &str = "outputs/research";

pub async fn run<L, S>(usecase: &ResearchUsecase<L, S>) -> Result<(), AgentCliError>
where
    L: LlmProvider,
    S: SearchProvider,
{
    println!("Research CLI");
    println!("type your query");

    let Some(query) = read_query()? else {
        return Ok(());
    };

    let output = usecase.run(RunResearchInput { query }).await?;
    println!("{}", output.reply);

    let saved_path = save_markdown_report(&output.reply)?;
    println!("saved report: {}", saved_path.display());

    Ok(())
}

fn read_query() -> Result<Option<String>, AgentCliError> {
    loop {
        print!("research > ");
        stdout().flush()?;

        let mut line = String::new();
        if stdin().read_line(&mut line)? == 0 {
            return Ok(None);
        }

        let query = line.trim().to_string();
        if query.is_empty() {
            continue;
        }

        return Ok(Some(query));
    }
}

fn save_markdown_report(markdown: &str) -> Result<PathBuf, AgentCliError> {
    let output_dir = PathBuf::from(RESEARCH_OUTPUT_DIR);
    fs::create_dir_all(&output_dir)?;

    let filename = format!("{}.md", Local::now().format("%Y-%m-%d_%H%M%S"));
    let path = output_dir.join(filename);

    fs::write(&path, markdown)?;

    Ok(path)
}
