use crate::{BoxError, DescribeError};

/// Why a [`Cli`](crate::cli::Cli) ran no job, or the job it ran failed.
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    /// No job name was given, and no default job is set.
    #[error("no job name given. Available: {}", available.join(", "))]
    MissingJob {
        /// The registered job names.
        available: Vec<String>,
    },

    /// No registered job has the given name.
    #[error("unknown job '{name}'. Available: {}", available.join(", "))]
    UnknownJob {
        /// The name given.
        name: String,
        /// The registered job names.
        available: Vec<String>,
    },

    /// The input is not JSON, or does not decode as the job's input.
    #[error("invalid input: {0}")]
    InvalidInput(#[source] BoxError),

    /// The job ran and failed.
    #[error("job failed: {0}")]
    JobFailed(#[source] BoxError),

    /// `TRACEL_DESCRIBE` names a path the definitions file could not be written to.
    #[error(transparent)]
    Describe(#[from] DescribeError),
}
