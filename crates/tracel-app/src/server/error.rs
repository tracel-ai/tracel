use crate::DescribeError;

/// Why a [`Server`](crate::server::Server) stopped serving, or could not start.
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    /// The server could not bind its address or serve on it.
    #[error("server error: {0}")]
    Io(#[from] std::io::Error),

    /// `TRACEL_DESCRIBE` names a path the definitions file could not be written to.
    #[error(transparent)]
    Describe(#[from] DescribeError),
}
