use std::error::Error;

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("no command name given and no default registered")]
    MissingDefault,

    #[error("unknown command '{name}'. Available: {}", available.join(", "))]
    UnknownCommand {
        name: String,
        available: Vec<String>,
    },

    #[error("validation failed: {0}")]
    ValidationFailed(#[source] Box<dyn Error + Send + Sync>),

    #[error("execution failed: {0}")]
    ExecutionFailed(#[source] Box<dyn Error + Send + Sync>),
}
