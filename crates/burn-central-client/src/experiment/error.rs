use burn::record::RecorderError;
use crate::websocket::WebSocketError;

#[derive(Debug, thiserror::Error)]
pub enum ExperimentError {
    #[error("Experiment is no longer active (likely already finished or dropped)")]
    InactiveExperiment,
    #[error("Experiment has already been finished")]
    AlreadyFinished,
    #[error("Experiment socket closed")]
    SocketClosed,
    #[error("Recorder error: {0}")]
    BurnRecorderError(#[from] RecorderError),
    #[error("WebSocket error: {0}")]
    WebSocketError(#[from] WebSocketError),
    #[error("Internal error: {0}")]
    InternalError(String),
}
