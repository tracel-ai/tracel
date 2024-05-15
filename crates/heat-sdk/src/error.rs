use thiserror::Error;

#[derive(Error, Debug)]
pub enum HeatSDKError {
    #[error("Server Timeout Error: {0}")]
    ServerTimeoutError(String),
    #[error("Server Error: {0}")]
    ServerError(String),
    #[error("Client Error: {0}")]
    ClientError(String),
    #[error("Unknown Error: {0}")]
    UnknownError(String),
}

impl From<reqwest::Error> for HeatSDKError {
    fn from(error: reqwest::Error) -> Self {
        HeatSDKError::ServerError(error.to_string())
    }
}
