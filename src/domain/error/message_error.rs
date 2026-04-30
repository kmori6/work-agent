use thiserror::Error;

#[derive(Debug, Error)]
pub enum MessageError {
    #[error("message contents must not be empty")]
    EmptyContents,

    #[error("message contents must not mix message and tool content")]
    MixedContentTypes,
}
