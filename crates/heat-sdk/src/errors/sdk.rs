use thiserror::Error;

use crate::{http::error::HeatHttpError, websocket::WebSocketError};

#[derive(Error, Debug)]
pub enum HeatSdkError {
    #[error("Invalid experiment number: {0}")]
    InvalidExperimentNumber(String),
    #[error("Invalid experiment path: {0}")]
    InvalidProjectPath(String),
    #[error("Invalid experiment path: {0}")]
    InvalidExperimentPath(String),
    #[error("Websocket Error: {0}")]
    WebSocketError(String),
    #[error("Macro Error: {0}")]
    MacroError(String),
    #[error("Failed to start experiment: {0}")]
    StartExperimentError(String),
    #[error("Failed to stop experiment: {0}")]
    StopExperimentError(String),
    #[error("Failed to create client: {0}")]
    CreateClientError(String),
    #[error("Failed to create remote metric logger: {0}")]
    CreateRemoteMetricLoggerError(String),

    #[error("File Read Error: {0}")]
    FileReadError(String),

    #[error("HTTP Error: {0}")]
    HttpError(HeatHttpError),

    #[error("Unknown Error: {0}")]
    UnknownError(String),
}

impl<T> From<std::sync::PoisonError<std::sync::MutexGuard<'_, T>>> for HeatSdkError {
    fn from(error: std::sync::PoisonError<std::sync::MutexGuard<'_, T>>) -> Self {
        HeatSdkError::UnknownError(error.to_string())
    }
}

impl From<WebSocketError> for HeatSdkError {
    fn from(error: WebSocketError) -> Self {
        HeatSdkError::WebSocketError(error.to_string())
    }
}

impl From<HeatHttpError> for HeatSdkError {
    fn from(error: HeatHttpError) -> Self {
        HeatSdkError::HttpError(error)
    }
}
