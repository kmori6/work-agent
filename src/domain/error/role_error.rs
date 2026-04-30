use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RoleError {
    #[error("unknown role: {0}")]
    Unknown(String),
}
