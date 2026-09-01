use std::error::Error;

use tracel_client::ClientError;

/// Errors produced while calling a Station.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StationError {
    /// The Station has no such resource, or will not say that it has one.
    #[error("the station has no such resource")]
    NotFound,

    /// The Station could not be reached.
    #[error("station transport failed: {0}")]
    Transport(String),

    /// The Station answered with something this client could not read.
    #[error("invalid station response: {0}")]
    InvalidResponse(String),

    /// Anything the client itself failed at, kept whole.
    #[error(transparent)]
    Other(Box<dyn Error + Send + Sync>),
}

impl StationError {
    /// Wraps a client error without interpreting it.
    pub fn other(error: impl Into<Box<dyn Error + Send + Sync>>) -> Self {
        Self::Other(error.into())
    }
}

impl From<ClientError> for StationError {
    fn from(error: ClientError) -> Self {
        if error.is_not_found() {
            return Self::NotFound;
        }
        Self::other(error)
    }
}
