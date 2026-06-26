use std::error::Error;

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("unknown job '{name}'. Available: {}", available.join(", "))]
    UnknownJob {
        name: String,
        available: Vec<String>,
    },

    #[error("job failed: {0}")]
    JobError(#[source] Box<dyn Error + Send + Sync>),

    #[error("server error: {0}")]
    IoError(#[from] std::io::Error),
}
