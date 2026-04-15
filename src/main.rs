use clap::Parser;
use dotenvy::dotenv;
use log::info;
use std::sync::Arc;
use work_agent::application::usecase::agent_usecase::{AgentUsecase, AppError};
use work_agent::presentation::cli::{Cli, Commands};

#[tokio::main]
async fn main() -> Result<(), AppError> {
    dotenv().ok();

    env_logger::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Agent => {
            info!("Starting agent...");
            let agent = Arc::new(AgentUsecase::new());
            agent.run().await?;
        }
    }

    Ok(())
}
