use thiserror::Error;

#[derive(Debug, Error)]
pub enum InferenceError {
    #[error("Inference handler execution failed: {0}")]
    HandlerExecutionFailed(anyhow::Error),
    #[error("Inference cancelled")]
    Cancelled,
    #[error("Inference thread panicked: {0}")]
    ThreadPanicked(String),
    #[error("Unexpected error: {0}")]
    Unexpected(String),
}
