use std::error::Error;

pub type BoxError = Box<dyn Error + Send + Sync>;

/// Errors that prevent the runner from starting.
///
/// Once serving, the runner never returns an error: connection losses are retried with backoff
/// and job failures are reported to the station as job outcomes.
#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
    #[error("invalid station url '{url}': {source}")]
    InvalidUrl {
        url: String,
        source: url::ParseError,
    },
    #[error("no jobs registered")]
    NoJobs,
}
