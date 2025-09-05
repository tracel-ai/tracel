use burn_central_client::model::ModelRegistryError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum InferenceError {
    #[error("Model loading failed: {0}")]
    ModelLoadingFailed(#[from] ModelProviderError),
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
pub enum ModelProviderError {
    #[error("Model registry error: {0}")]
    ModelLoadingFailed(#[from] ModelRegistryError),
}

pub type ModelProviderResult<M> = Result<M, ModelProviderError>;
