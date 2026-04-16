use clap::Parser;
use dotenvy::dotenv;
use log::info;
use std::sync::Arc;
use work_agent::domain::service::agent_service::AgentService;
use work_agent::domain::service::tool_service::ToolExecutor;
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
            let tool_executor = ToolExecutor::new(vec![Arc::new(WebSearchTool::from_env()?)]);
            let agent_service = AgentService::new(llm_client, tool_executor);
            let usecase = AgentUsecase::new(agent_service);
            agent_cli::run(&usecase).await?;
        }
    }

    Ok(())
}
