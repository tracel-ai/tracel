use burn_central_client::model::ModelRegistryError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum InferenceError {
    #[error("Model loading failed: {0}")]
    ModelInitFailed(#[from] InitError),
    #[error("Inference handler execution failed: {0}")]
    HandlerExecutionFailed(anyhow::Error),
    #[error("Inference cancelled")]
    Cancelled,
    #[error("Unexpected error: {0}")]
    Unexpected(String),
    #[error("Inference thread panicked: {0}")]
    ThreadPanicked(String),
}

#[derive(Debug, Error)]
pub enum InitError {
    #[error("Model registry error: {0}")]
    ModelLoadingFailed(#[from] ModelRegistryError),
}
