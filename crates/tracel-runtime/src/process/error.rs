use tracel_job_proto::ConfigError;

use crate::process::describe::DescribeError;
use crate::{BoxError, PrepareError};

/// The exit code of a job that failed, or of a definitions file that could not be written.
const FAILED: u8 = 1;
/// The exit code of a command line that names no registered job, or gives an unusable flag or
/// input.
const USAGE: u8 = 2;
/// The exit code of a job that was asked to stop.
pub const STOPPED: u8 = 130;

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

    /// The job was asked to stop.
    #[error("the job was asked to stop")]
    Stopped,

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
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::MissingJob { .. }
            | Self::UnknownJob { .. }
            | Self::Usage(_)
            | Self::Config(_)
            | Self::InvalidInput(_) => USAGE,
            Self::JobFailed(_) | Self::Describe(_) => FAILED,
            Self::Stopped => STOPPED,
        }
    }

    /// Prints why to stderr: a usage error as clap renders it, with the usage that follows, and a
    /// stop request as a warning, since a job asked to stop may still have completed.
    pub fn print(&self) {
        match self {
            Self::Usage(error) => {
                let _ = error.print();
            }
            Self::Stopped => eprintln!("warning: {self}"),
            error => eprintln!("error: {error}"),
        }
    }
}
