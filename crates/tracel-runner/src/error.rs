use std::error::Error;

pub type BoxError = Box<dyn Error + Send + Sync>;

/// Fatal runner errors.
///
/// Transient connection losses are retried with backoff and job failures are reported to the
/// station as job outcomes; the runner returns only when it cannot start or when the station
/// permanently rejects its registration.
#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
    #[error("invalid station url '{url}': {source}")]
    InvalidUrl {
        url: String,
        source: url::ParseError,
    },
    #[error("no jobs registered")]
    NoJobs,
    #[error("station rejected runner registration: {0}")]
    Registration(String),
}
