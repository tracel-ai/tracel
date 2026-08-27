use crate::VersionId;

/// Errors produced by model operations.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ModelsError {
    /// No model by that name.
    #[error("model '{name}' was not found")]
    ModelNotFound {
        /// Requested model name.
        name: String,
    },
    /// No such version of that model.
    #[error("version '{id}' of model '{model}' was not found")]
    VersionNotFound {
        /// Requested model name.
        model: String,
        /// Requested version identity.
        id: VersionId,
    },
    /// The transfer was cancelled.
    #[error("model transfer cancelled")]
    Cancelled,
    /// A file in the version is published under a path that cannot be used.
    #[error("invalid model file path: {0}")]
    InvalidPath(String),
    /// A file in the version is published with something that is not a checksum.
    #[error("invalid model file checksum: {0}")]
    InvalidChecksum(String),
    /// A file did not match the size or checksum it was published with.
    #[error("file '{rel_path}' does not match what was published: {problem}")]
    Verification {
        /// Path of the file within the version.
        rel_path: String,
        /// What did not match.
        problem: String,
    },
    /// The model could not be written out.
    #[error("writing the model out failed: {0}")]
    Output(String),
    /// The model could not be decoded.
    #[error("model decoding failed: {0}")]
    Decode(String),
    /// Any other failure, reported by whatever serves these models.
    #[error(transparent)]
    Other(Box<dyn std::error::Error + Send + Sync>),
}

impl ModelsError {
    /// Wraps a failure the model domain has no meaning for.
    pub fn other(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Self {
        Self::Other(error.into())
    }

    /// Returns whether a requested model or version does not exist.
    pub fn is_not_found(&self) -> bool {
        matches!(
            self,
            Self::ModelNotFound { .. } | Self::VersionNotFound { .. }
        )
    }

    /// Returns whether a file did not match what was published, or could not be accepted.
    pub fn is_verification(&self) -> bool {
        matches!(
            self,
            Self::Verification { .. } | Self::InvalidPath(_) | Self::InvalidChecksum(_)
        )
    }

    /// Returns whether the transfer was cancelled.
    pub fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }
}
