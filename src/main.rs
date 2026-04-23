use clap::Parser;
use commander::domain::service::agent_service::AgentService;
use commander::domain::service::context_service::ContextService;
use commander::domain::service::deep_research_service::DeepResearchService;
use commander::domain::service::tool_service::ToolExecutor;
use commander::infrastructure::persistence::postgres_chat_message_repository::PostgresChatMessageRepository;
use commander::infrastructure::persistence::postgres_chat_session_repository::PostgresChatSessionRepository;
use commander::infrastructure::persistence::postgres_token_usage_repository::PostgresTokenUsageRepository;
use commander::infrastructure::persistence::postgres_tool_approval_repository::PostgresToolApprovalRepository;
use commander::infrastructure::search::tavily_search_provider::TavilySearchProvider;
use commander::infrastructure::tool::asr_tool::AsrTool;
use commander::infrastructure::tool::file_edit_tool::FileEditTool;
use commander::infrastructure::tool::file_read_tool::FileReadTool;
use commander::infrastructure::tool::file_search_tool::FileSearchTool;
use commander::infrastructure::tool::file_write_tool::FileWriteTool;
use commander::infrastructure::tool::ocr_tool::OcrTool;
use commander::infrastructure::tool::shell_exec_tool::ShellExecTool;
use commander::infrastructure::tool::text_search_tool::TextSearchTool;
use commander::infrastructure::tool::web_fetch_tool::WebFetchTool;
use commander::infrastructure::tool::web_search_tool::WebSearchTool;
use commander::{
    application::usecase::{
        agent_usecase::AgentUsecase, digest_usecase::DigestUsecase,
        research_usecase::ResearchUsecase, survey_usecase::SurveyUsecase,
    },
    infrastructure::llm::bedrock_llm_provider::BedrockLlmProvider,
    presentation::{
        cli::{Cli, Commands, agent_cli, digest_cli, research_cli, survey_cli},
        error::agent_cli_error::AgentCliError,
    },
};
use dotenvy::dotenv;
use log::info;
use sqlx::PgPool;
use std::env;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), AgentCliError> {
    dotenv().ok();
    env_logger::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Agent => {
            info!("Starting agent...");

            let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
            let pool = PgPool::connect(&database_url).await?;

            let llm_client = BedrockLlmProvider::from_default_config().await;
            let workspace_root = env::current_dir()?;

            let tool_executor = ToolExecutor::new(vec![
                Arc::new(AsrTool::from_env(workspace_root.clone())?),
                Arc::new(FileSearchTool::new(workspace_root.clone(), 200)?),
                Arc::new(OcrTool::new(workspace_root.clone())?),
                Arc::new(ShellExecTool::new(workspace_root.clone())?),
                Arc::new(FileWriteTool::new(workspace_root.clone())?),
                Arc::new(FileEditTool::new(workspace_root.clone(), 1_048_576)?),
                Arc::new(FileReadTool::new(workspace_root.clone(), 1_048_576)?),
                Arc::new(TextSearchTool::new(workspace_root, 1_048_576, 200, 10)?),
                Arc::new(WebFetchTool::new()?),
                Arc::new(WebSearchTool::from_env()?),
            ]);

            let context_service = ContextService::new(llm_client.clone());
            let agent_service = AgentService::new(llm_client, tool_executor);

            let chat_session_repository = PostgresChatSessionRepository::new(pool.clone());
            let chat_message_repository = PostgresChatMessageRepository::new(pool.clone());
            let token_usage_repository = PostgresTokenUsageRepository::new(pool.clone());
            let tool_approval_repository = PostgresToolApprovalRepository::new(pool);

            let usecase = AgentUsecase::new(
                agent_service,
                context_service,
                chat_session_repository,
                chat_message_repository,
                token_usage_repository,
                tool_approval_repository,
            );

            agent_cli::run(&usecase).await?;
        }
        Commands::Research => {
            info!("Starting research...");
            let llm_client = BedrockLlmProvider::from_default_config().await;
            let search_provider = TavilySearchProvider::from_env()?;
            let usecase =
                ResearchUsecase::new(DeepResearchService::new(llm_client, search_provider));
            research_cli::run(&usecase).await?;
        }
        Commands::Survey { source, output } => {
            info!("Starting survey...");
            let llm_client = BedrockLlmProvider::from_default_config().await;
            let usecase = SurveyUsecase::new(llm_client);
            survey_cli::run(&usecase, &source, output).await?;
        }
        Commands::Digest { date, output } => {
            info!("Starting digest...");
            let llm_client = BedrockLlmProvider::from_default_config().await;
            let usecase = DigestUsecase::new(llm_client);
            digest_cli::run(&usecase, date, output).await?;
        }
    }

    Ok(())
}
