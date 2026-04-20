use crate::domain::error::deep_research_error::DeepResearchError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ResearchUsecaseError {
    #[error("failed to execute research use case: {0}")]
    DeepResearch(#[from] DeepResearchError),
}
