use clap::Parser;
use dotenvy::dotenv;
use log::info;
use std::env;
use std::sync::Arc;
use work_agent::domain::service::agent_service::AgentService;
use work_agent::domain::service::tool_service::ToolExecutor;
use work_agent::infrastructure::tool::asr_tool::AsrTool;
use work_agent::infrastructure::tool::file_search_tool::FileSearchTool;
use work_agent::infrastructure::tool::read_file_tool::ReadFileTool;
use work_agent::infrastructure::tool::research_tool::ResearchTool;
use work_agent::infrastructure::tool::text_file_edit_tool::TextFileEditTool;
use work_agent::infrastructure::tool::text_file_write_tool::TextFileWriteTool;
use work_agent::infrastructure::tool::text_search_tool::TextSearchTool;
use work_agent::infrastructure::tool::web_fetch_tool::WebFetchTool;
use work_agent::infrastructure::tool::web_search_tool::WebSearchTool;
use work_agent::{
    application::usecase::agent_usecase::AgentUsecase,
    infrastructure::llm::bedrock_llm_provider::BedrockLlmProvider,
    presentation::{
        cli::{Cli, Commands, agent_cli},
        error::agent_cli_error::AgentCliError,
    },
};

#[tokio::main]
async fn main() -> Result<(), AgentCliError> {
    dotenv().ok();

    env_logger::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Agent => {
            info!("Starting agent...");
            let llm_client = BedrockLlmProvider::from_default_config().await;
            let research_tool = ResearchTool::from_env(llm_client.clone())?;
            let workspace_root = env::current_dir()?;
            let tool_executor = ToolExecutor::new(vec![
                Arc::new(AsrTool::from_env(workspace_root.clone())?),
                Arc::new(FileSearchTool::new(workspace_root.clone(), 200)?),
                Arc::new(research_tool),
                Arc::new(TextFileWriteTool::new(workspace_root.clone())?),
                Arc::new(TextFileEditTool::new(workspace_root.clone(), 1_048_576)?),
                Arc::new(ReadFileTool::new(workspace_root.clone(), 1_048_576)?),
                Arc::new(TextSearchTool::new(workspace_root, 1_048_576, 200, 10)?),
                Arc::new(WebFetchTool::new()?),
                Arc::new(WebSearchTool::from_env()?),
            ]);
            let agent_service = AgentService::new(llm_client, tool_executor);
            let usecase = AgentUsecase::new(agent_service);
            agent_cli::run(&usecase).await?;
        }
    }

    Ok(())
}
