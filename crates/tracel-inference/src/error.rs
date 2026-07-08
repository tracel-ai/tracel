/// Error returned while setting up or running an inference job.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct InferenceError {
    /// Human-readable description of what went wrong.
    pub message: String,
    /// The underlying cause, if any.
    #[source]
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl InferenceError {
    /// Create an error with just a message.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source: None,
        }
    }

    /// Create an error that wraps an underlying cause.
    pub fn with_source(
        message: impl Into<String>,
        source: impl Into<Box<dyn std::error::Error + Send + Sync>>,
    ) -> Self {
        Self {
            message: message.into(),
            source: Some(source.into()),
        }
    }
}
