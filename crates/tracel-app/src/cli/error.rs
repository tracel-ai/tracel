use std::error::Error;

use crate::job_register::JobRegisterError;

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("no job name given and no default registered")]
    MissingDefault,

    #[error("unknown job '{name}'. Available: {}", available.join(", "))]
    UnknownJob {
        name: String,
        available: Vec<String>,
    },

    #[error("validation failed: {0}")]
    ValidationFailed(#[source] Box<dyn Error + Send + Sync>),

    #[error("execution failed: {0}")]
    ExecutionFailed(#[source] Box<dyn Error + Send + Sync>),
}

impl From<JobRegisterError> for CliError {
    fn from(err: JobRegisterError) -> Self {
        match err {
            JobRegisterError::UnknownJob { name, available } => {
                CliError::UnknownJob { name, available }
            }
            JobRegisterError::ValidationFailed(e) => CliError::ValidationFailed(e),
            JobRegisterError::ExecutionFailed(e) => CliError::ExecutionFailed(e),
        }
    }
}
