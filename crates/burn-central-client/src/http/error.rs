use reqwest::StatusCode;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum BurnCentralHttpError {
    #[error("Bad session id")]
    BadSessionId,
    #[error("HTTP {status}: {body}")]
    HttpError {
        status: StatusCode,
        body: String,
    },
    #[error("Unknown Error: {0}")]
    UnknownError(String),
}
