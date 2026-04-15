pub type AppError = Box<dyn std::error::Error + Send + Sync>;

pub struct AgentUsecase {}

impl Default for AgentUsecase {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentUsecase {
    pub fn new() -> Self {
        Self {}
    }

    pub async fn run(&self) -> Result<(), AppError> {
        Ok(())
    }
}
