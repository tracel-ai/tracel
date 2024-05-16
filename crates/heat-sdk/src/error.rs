use thiserror::Error;

#[derive(Error, Debug)]
pub enum HeatSDKError {
    #[error("Server Timeout Error: {0}")]
    ServerTimeoutError(String),
    #[error("Server Error: {0}")]
    ServerError(String),
    #[error("Client Error: {0}")]
    ClientError(String),
    #[error("Websocket Error: {0}")]
    WebSocketError(String),
    #[error("Unknown Error: {0}")]
    UnknownError(String),
}

impl From<reqwest::Error> for HeatSDKError {
    fn from(error: reqwest::Error) -> Self {
        HeatSDKError::ServerError(error.to_string())
    }
}

impl<T> From<std::sync::PoisonError<std::sync::MutexGuard<'_, T>>> for HeatSDKError {
    fn from(error: std::sync::PoisonError<std::sync::MutexGuard<'_, T>>) -> Self {
        HeatSDKError::ClientError(error.to_string())
    }
}
