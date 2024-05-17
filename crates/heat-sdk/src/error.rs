use thiserror::Error;

#[derive(Error, Debug)]
pub enum HeatSdkError {
    #[error("Server Timeout Error: {0}")]
    ServerTimeoutError(String),
    #[error("Server Error: {0}")]
    ServerError(String),
    #[error("Client Error: {0}")]
    ClientError(String),
    #[error("Unknown Error: {0}")]
    UnknownError(String),
}

impl From<reqwest::Error> for HeatSdkError {
    fn from(error: reqwest::Error) -> Self {
        HeatSdkError::ServerError(error.to_string())
    }
}
