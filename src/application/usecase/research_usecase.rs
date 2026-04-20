use crate::application::error::research_usecase_error::ResearchUsecaseError;
use crate::domain::port::llm_provider::LlmProvider;
use crate::domain::port::search_provider::SearchProvider;
use crate::domain::service::deep_research_service::DeepResearchService;

#[derive(Debug)]
pub struct RunResearchInput {
    pub query: String,
}

#[derive(Debug)]
pub struct RunResearchOutput {
    pub reply: String,
}

pub struct ResearchUsecase<L, S> {
    deep_research_service: DeepResearchService<L, S>,
}

impl<L, S> ResearchUsecase<L, S>
where
    L: LlmProvider,
    S: SearchProvider,
{
    pub fn new(deep_research_service: DeepResearchService<L, S>) -> Self {
        Self {
            deep_research_service,
        }
    }

    pub async fn run(
        &self,
        input: RunResearchInput,
    ) -> Result<RunResearchOutput, ResearchUsecaseError> {
        let reply = self.deep_research_service.research(input.query).await?;
        Ok(RunResearchOutput { reply })
    }
}
