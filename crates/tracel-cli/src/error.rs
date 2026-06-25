use std::error::Error;

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("unknown job '{name}'. Available: {}", available.join(", "))]
    UnknownJob { name: String, available: Vec<String> },

    #[error("no job name given and no default registered")]
    MissingDefault,

    #[error("invalid config: {0}")]
    ConfigError(#[source] Box<dyn Error + Send + Sync>),

    #[error("job failed: {0}")]
    JobError(#[source] Box<dyn Error + Send + Sync>),
}
