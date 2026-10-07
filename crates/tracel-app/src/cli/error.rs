use super::report::ReportError;
use crate::{BoxError, DescribeError};

/// The exit code of a job that failed, or of a definitions file or a run report that could not be
/// written.
const FAILED: u8 = 1;
/// The exit code of a command line that names no registered job, or gives an unusable flag or
/// input.
const USAGE: u8 = 2;
/// The exit code of a job that was asked to stop.
pub const STOPPED: u8 = 130;

/// Why a [`Cli`](crate::cli::Cli) ran no job, or the job it ran did not complete.
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

    /// The command line does not parse: an unknown flag, a flag value of the wrong type, a
    /// required flag left out, or an input or `--config` file that is not JSON.
    #[error(transparent)]
    Usage(clap::Error),

    /// The input does not decode as the job's input.
    #[error("invalid input: {0}")]
    InvalidInput(#[source] BoxError),

    /// The job ran and failed.
    #[error("job failed: {0}")]
    JobFailed(#[source] BoxError),

    /// The job was asked to stop.
    #[error("the job was asked to stop")]
    Stopped,

    /// `TRACEL_DESCRIBE` names a path the definitions file could not be written to.
    #[error(transparent)]
    Describe(#[from] DescribeError),

    /// `TRACEL_REPORT_FILE` names a path the run report could not be written to when the job
    /// started, so it did not run.
    #[error(transparent)]
    Report(#[from] ReportError),
}

impl CliError {
    /// The exit code the process ends with.
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::MissingJob { .. }
            | Self::UnknownJob { .. }
            | Self::Usage(_)
            | Self::InvalidInput(_) => USAGE,
            Self::JobFailed(_) | Self::Describe(_) | Self::Report(_) => FAILED,
            Self::Stopped => STOPPED,
        }
    }

    /// Prints why to stderr, a usage error as clap renders it, with the usage that follows.
    pub fn print(&self) {
        match self {
            Self::Usage(error) => {
                let _ = error.print();
            }
            error => eprintln!("error: {error}"),
        }
    }
}
