use burn::record::RecorderError;

use crate::artifacts::ArtifactBuilderError;

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
    #[error("Artifact error: {0}")]
    BurnArtifactError(#[from] ArtifactBuilderError),
    #[error("Failed to connect to the server: {0}")]
    ConnectionFailed(String),
    #[error("Internal error: {0}")]
    InternalError(String),
}
