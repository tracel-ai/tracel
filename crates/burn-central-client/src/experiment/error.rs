use crate::websocket::WebSocketError;
use burn::record::RecorderError;

#[derive(Debug, thiserror::Error)]
pub enum ExperimentTrackerError {
    #[error("Experiment is no longer active (likely already finished or dropped)")]
    InactiveExperiment,
    #[error("Experiment has already been finished")]
    AlreadyFinished,
    #[error("Experiment socket closed")]
    SocketClosed,
    #[error("Recorder error: {0}")]
    BurnRecorderError(#[from] RecorderError),
    #[error("Failed to connect to the server: {0}")]
    ConnectionFailed(String),
    #[error("Internal error: {0}")]
    InternalError(String),
}
