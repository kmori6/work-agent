use std::{env, sync::Arc};

use clap::Parser;
use dotenvy::dotenv;
use log::info;
use sqlx::PgPool;

use commander::application::usecase::{
    agent_usecase::AgentUsecase, digest_usecase::DigestUsecase, research_usecase::ResearchUsecase,
    survey_usecase::SurveyUsecase, tool_execution_rule_usecase::ToolExecutionRuleUsecase,
};
use commander::domain::service::{
    agent_service::AgentService, compaction_service::CompactionService,
    deep_research_service::DeepResearchService, instruction_service::InstructionService,
    memory_index_service::MemoryIndexService, tool_service::ToolService,
};
use commander::infrastructure::{
    embedding::bedrock_embedding_provider::BedrockEmbeddingProvider,
    llm::bedrock_llm_provider::BedrockLlmProvider,
    persistence::{
        postgres_awaiting_tool_approval_repository::PostgresAwaitingToolApprovalRepository,
        postgres_chat_message_repository::PostgresChatMessageRepository,
        postgres_chat_session_repository::PostgresChatSessionRepository,
        postgres_memory_index_repository::PostgresMemoryIndexRepository,
        postgres_token_usage_repository::PostgresTokenUsageRepository,
        postgres_tool_approval_repository::PostgresToolApprovalRepository,
        postgres_tool_execution_rule_repository::PostgresToolExecutionRuleRepository,
    },
    search::tavily_search_provider::TavilySearchProvider,
    tool::{
        asr_tool::AsrTool, file_edit_tool::FileEditTool, file_read_tool::FileReadTool,
        file_search_tool::FileSearchTool, file_write_tool::FileWriteTool,
        memory_search_tool::MemorySearchTool, memory_write_tool::MemoryWriteTool,
        ocr_tool::OcrTool, shell_exec_tool::ShellExecTool, text_search_tool::TextSearchTool,
        web_fetch_tool::WebFetchTool, web_search_tool::WebSearchTool,
    },
};
use commander::presentation::{
    cli::{Cli, Commands, agent_cli, chat_cli, digest_cli, research_cli, serve_cli, survey_cli},
    error::agent_cli_error::AgentCliError,
};

#[tokio::main]
async fn main() -> Result<(), AgentCliError> {
    dotenv().ok();
    env_logger::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Serve { addr } => {
            info!("Starting server on {}", addr);
            serve_cli::run(addr).await?;
        }
        Commands::Chat => {
            info!("Starting chat CLI...");
            chat_cli::run().await?;
        }
        Commands::Agent => {
            info!("Starting agent...");

            let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
            let pool = PgPool::connect(&database_url).await?;

            let llm_client = BedrockLlmProvider::from_default_config().await;
            let embedding_provider =
                Arc::new(BedrockEmbeddingProvider::from_default_config().await);
            let memory_index_repository =
                Arc::new(PostgresMemoryIndexRepository::new(pool.clone()));
            let memory_index_service = Arc::new(MemoryIndexService::new(
                embedding_provider,
                memory_index_repository,
            ));
            let workspace_root = env::current_dir()?;
            let instruction_service = InstructionService::new(workspace_root.clone());

            let tool_execution_rule_repository =
                PostgresToolExecutionRuleRepository::new(pool.clone());
            let tool_service = ToolService::new(
                vec![
                    Arc::new(AsrTool::from_env(workspace_root.clone())?),
                    Arc::new(FileSearchTool::new(workspace_root.clone(), 200)?),
                    Arc::new(OcrTool::new(workspace_root.clone())?),
                    Arc::new(ShellExecTool::new(workspace_root.clone())?),
                    Arc::new(MemorySearchTool::new(memory_index_service.clone())),
                    Arc::new(MemoryWriteTool::new(
                        workspace_root.clone(),
                        memory_index_service.clone(),
                    )?),
                    Arc::new(FileWriteTool::new(workspace_root.clone())?),
                    Arc::new(FileEditTool::new(workspace_root.clone(), 1_048_576)?),
                    Arc::new(FileReadTool::new(workspace_root.clone(), 1_048_576)?),
                    Arc::new(TextSearchTool::new(workspace_root, 1_048_576, 200, 10)?),
                    Arc::new(WebFetchTool::new()?),
                    Arc::new(WebSearchTool::from_env()?),
                ],
                Arc::new(tool_execution_rule_repository.clone()),
            );

            let context_service = CompactionService::new(llm_client.clone());
            let agent_service = AgentService::new(llm_client, tool_service);

            let chat_session_repository = PostgresChatSessionRepository::new(pool.clone());
            let chat_message_repository = PostgresChatMessageRepository::new(pool.clone());
            let token_usage_repository = PostgresTokenUsageRepository::new(pool.clone());
            let tool_approval_repository = PostgresToolApprovalRepository::new(pool.clone());
            let awaiting_tool_approval_repository =
                PostgresAwaitingToolApprovalRepository::new(pool.clone());
            let tool_execution_rule_repository =
                PostgresToolExecutionRuleRepository::new(pool.clone());
            let tool_execution_rule_usecase =
                ToolExecutionRuleUsecase::new(tool_execution_rule_repository.clone());

            let usecase = AgentUsecase::new(
                agent_service,
                instruction_service,
                context_service,
                chat_session_repository,
                chat_message_repository,
                token_usage_repository,
                tool_approval_repository,
                awaiting_tool_approval_repository,
            );

            agent_cli::run(&usecase, &tool_execution_rule_usecase).await?;
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
