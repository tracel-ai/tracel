use crate::artifacts::ArtifactError;

#[derive(Debug, thiserror::Error)]
pub enum ExperimentTrackerError {
    #[error("Experiment is no longer active (likely already finished or dropped)")]
    InactiveExperiment,
    #[error("Experiment has already been finished")]
    AlreadyFinished,
    #[error("Experiment socket closed")]
    SocketClosed,
    #[error("Artifact error: {0}")]
    ArtifactError(#[from] ArtifactError),
    #[error("Failed to connect to the server: {0}")]
    ConnectionFailed(String),
    #[error("Internal error: {0}")]
    InternalError(String),
}
