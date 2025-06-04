use reqwest::StatusCode;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum BurnCentralHttpError {
    #[error("Bad session id")]
    BadSessionId,
    #[error("Http Error {0}: {1}")]
    HttpError(StatusCode, String),

    #[error("Unknown Error: {0}")]
    UnknownError(String),
}
