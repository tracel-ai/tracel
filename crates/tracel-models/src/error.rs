use crate::VersionSpec;

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
    #[error("model '{model}' has no {version}")]
    VersionNotFound {
        /// Requested model name.
        model: String,
        /// Version that was asked for.
        version: VersionSpec,
    },
    /// No alias by that name on that model.
    #[error("model '{model}' has no alias '{alias}'")]
    AliasNotFound {
        /// Requested model name.
        model: String,
        /// Alias that was asked for.
        alias: String,
    },
    /// The version exists, but its files are still arriving or never arrived.
    #[error("{version} of model '{model}' is not ready")]
    VersionNotReady {
        /// Requested model name.
        model: String,
        /// Version that was asked for.
        version: VersionSpec,
    },
    /// The version was deleted: it stays readable by number, but its files are gone.
    #[error("{version} of model '{model}' has been deleted")]
    VersionDeleted {
        /// Requested model name.
        model: String,
        /// Version that was asked for.
        version: VersionSpec,
    },
    /// A name that cannot be an alias: up to 64 lowercase letters, digits, `.`, `_` or `-`,
    /// starting with a letter or digit, and neither `latest` nor a version number.
    #[error("'{0}' is not an alias name")]
    InvalidAlias(String),
    /// The backend refused a change that conflicts with the model's current state, such as
    /// completing an upload whose parts did not all arrive.
    #[error("the change conflicts with the current state of model '{model}': {code}")]
    Conflict {
        /// Requested model name.
        model: String,
        /// The backend's name for the conflict.
        code: String,
    },
    /// The transfer was cancelled.
    #[error("model transfer cancelled")]
    Cancelled,
    /// Communication with the backend or a file endpoint failed.
    #[error("model transport failed: {0}")]
    Transport(String),
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

    /// Returns whether a requested model, version or alias does not exist.
    pub fn is_not_found(&self) -> bool {
        matches!(
            self,
            Self::ModelNotFound { .. } | Self::VersionNotFound { .. } | Self::AliasNotFound { .. }
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
