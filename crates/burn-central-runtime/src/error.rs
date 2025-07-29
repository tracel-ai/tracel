use burn_central_client::BurnCentralError;
use burn_central_client::experiment::ExperimentTrackerError;

#[derive(thiserror::Error, Debug)]
pub enum RuntimeError {
    #[error("Handler '{0}' not found")]
    HandlerNotFound(String),
    #[error("Burn Central API call failed: {0}")]
    BurnCentralError(#[from] BurnCentralError),
    #[error("Experiment API call failed: {0}")]
    ExperimentApiFailed(#[from] ExperimentTrackerError),
    #[error("Handler execution failed: {0}")]
    HandlerFailed(anyhow::Error),
    #[error("Ambiguous target '{0}'. Found multiple handlers: {1:?}")]
    AmbiguousHandlerName(String, Vec<String>),
}