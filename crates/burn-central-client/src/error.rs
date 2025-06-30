use thiserror::Error;

use crate::{http::error::BurnCentralHttpError, websocket::WebSocketError};

#[derive(Error, Debug)]
pub enum BurnCentralClientError {
    #[error("Invalid experiment number: {0}")]
    InvalidExperimentNumber(String),
    #[error("Invalid experiment path: {0}")]
    InvalidProjectPath(String),
    #[error("Invalid experiment path: {0}")]
    InvalidExperimentPath(String),
    #[error("Websocket Error: {0}")]
    WebSocketError(#[from] WebSocketError),
    #[error("Macro Error: {0}")]
    MacroError(String),
    #[error("Failed to start experiment: {0}")]
    StartExperimentError(String),
    #[error("Failed to stop experiment: {0}")]
    StopExperimentError(String),
    #[error("Invalid credentials: {0}")]
    InvalidCredentialsError(String),
    #[error("Failed to reach server: {0}")]
    ServerConnectionError(String),
    #[error("Failed to create remote metric logger: {0}")]
    CreateRemoteMetricLoggerError(String),
    #[error("Failed to authenticate user: {0}")]
    AuthenticationError(String),
    #[error("Invalid project id: {0}")]
    InvalidProjectError(String),
    #[error("Failed to set project: {0}")]
    SetProjectError(String),
    #[error("Failed to upload project: {0}")]
    UploadProjectVersionError(String),
    #[error("Failed to start remote job: {0}")]
    StartRemoteJobError(String),
    #[error("Failed to create project: {0}")]
    CreateProjectError(String),
    #[error("Failed to get project: {0}")]
    GetProjectError(String),

    #[error("File Read Error: {0}")]
    FileReadError(String),

    #[error(transparent)]
    HttpError(#[from] BurnCentralHttpError),

    #[error("Unknown Error: {0}")]
    UnknownError(String),
}

impl<T> From<std::sync::PoisonError<std::sync::MutexGuard<'_, T>>> for BurnCentralClientError {
    fn from(error: std::sync::PoisonError<std::sync::MutexGuard<'_, T>>) -> Self {
        BurnCentralClientError::UnknownError(error.to_string())
    }
}
