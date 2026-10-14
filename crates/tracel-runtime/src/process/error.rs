use tracel_job::ExitCode;
use tracel_job::cli::ConfigError;

use crate::process::describe::DescribeError;
use crate::{BoxError, PrepareError};

/// Why [`Jobs::run`](crate::Jobs::run) ran no job, or the job it ran did not complete.
#[derive(Debug, thiserror::Error)]
pub enum RunError {
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

    /// The command line does not parse: an unknown flag, a flag value of the wrong type, a
    /// required flag left out, or an input that is not JSON.
    #[error(transparent)]
    Usage(clap::Error),

    /// The `--config` file cannot be read or is not JSON.
    #[error(transparent)]
    Config(ConfigError),

    /// The input does not decode as the job's input.
    #[error("invalid input: {0}")]
    InvalidInput(#[source] serde_json::Error),

    /// The job ran and failed.
    #[error("job failed: {0}")]
    JobFailed(#[source] BoxError),

    /// `TRACEL_DESCRIBE` names a path the definitions file could not be written to.
    #[error(transparent)]
    Describe(#[from] DescribeError),
}

impl From<PrepareError> for RunError {
    fn from(error: PrepareError) -> Self {
        match error {
            PrepareError::UnknownJob { name, available } => Self::UnknownJob { name, available },
            PrepareError::InvalidInput(error) => Self::InvalidInput(error),
        }
    }
}

impl RunError {
    /// The exit code the process ends with.
    pub fn exit_code(&self) -> ExitCode {
        match self {
            Self::MissingJob { .. }
            | Self::UnknownJob { .. }
            | Self::Usage(_)
            | Self::Config(_)
            | Self::InvalidInput(_) => ExitCode::Rejected,
            Self::JobFailed(_) | Self::Describe(_) => ExitCode::Failed,
        }
    }

    /// Prints why to stderr: a usage error as clap renders it, with the usage that follows.
    pub fn print(&self) {
        match self {
            Self::Usage(error) => {
                let _ = error.print();
            }
            error => eprintln!("error: {error}"),
        }
    }
}
