use thiserror::Error;

use crate::websocket::WebSocketError;

#[derive(Error, Debug)]
pub enum HeatSdkError {
    #[error("Server Timeout Error: {0}")]
    ServerTimeoutError(String),
    #[error("Server Error: {0}")]
    ServerError(String),
    #[error("Client Error: {0}")]
    ClientError(String),
    #[error("Websocket Error: {0}")]
    WebSocketError(String),
    #[error("Macro Error: {0}")]
    MacroError(String),
    #[error("Unknown Error: {0}")]
    UnknownError(String),
}

impl<T> From<std::sync::PoisonError<std::sync::MutexGuard<'_, T>>> for HeatSdkError {
    fn from(error: std::sync::PoisonError<std::sync::MutexGuard<'_, T>>) -> Self {
        HeatSdkError::ClientError(error.to_string())
    }
}

impl From<WebSocketError> for HeatSdkError {
    fn from(error: WebSocketError) -> Self {
        HeatSdkError::WebSocketError(error.to_string())
    }
}
