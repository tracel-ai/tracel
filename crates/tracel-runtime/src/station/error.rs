/// Fatal runner errors.
///
/// Transient connection losses are retried with backoff and job failures are reported to the
/// station as job outcomes; the runner returns only when it cannot start or when the station
/// permanently rejects its registration.
#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
    /// The station URL does not parse.
    #[error("invalid station url '{url}': {source}")]
    InvalidUrl {
        /// The URL given.
        url: String,
        /// Why it does not parse.
        source: url::ParseError,
    },
    /// The runner was given no jobs to serve.
    #[error("no jobs registered")]
    NoJobs,
    /// The station rejected the runner's registration for good, with the status and body it
    /// answered with.
    #[error("station rejected runner registration: {0}")]
    Registration(String),
}
