use tracel_artifact::download::DownloadError;

use crate::VersionId;

/// Errors produced by model domain operations and verified transfers.
#[derive(Debug, thiserror::Error)]
pub enum ModelsError {
    /// The capability's backend-defined scope does not exist or is not visible.
    #[error("the model scope was not found or is not visible")]
    ScopeNotFound,
    /// The named model does not exist in the capability's scope.
    #[error("model '{name}' not found")]
    ModelNotFound {
        /// Requested model name.
        name: String,
    },
    /// The opaque version identity does not belong to the named model.
    #[error("version '{id}' of model '{model}' not found")]
    VersionNotFound {
        /// Requested model name.
        model: String,
        /// Requested opaque version identity.
        id: VersionId,
    },
    /// The backend rejected or could not authenticate the request.
    #[error("model authentication failed: {0}")]
    Authentication(String),
    /// Communication with the model backend or file source failed.
    #[error("model transport failed: {0}")]
    Transport(String),
    /// A backend response could not be represented by the model domain.
    #[error("invalid model response: {0}")]
    InvalidResponse(String),
    /// A downloaded file failed path, size, or checksum verification.
    #[error("model file verification failed: {0}")]
    Verification(#[source] DownloadError),
    /// Temporary verified staging could not be created or read.
    #[error("model staging failed: {0}")]
    Staging(String),
    /// Verified files could not be written to the caller's destination.
    #[error("model destination failed: {0}")]
    Destination(String),
    /// A backend-specific cache could not read or persist verified bytes.
    #[error("model cache failed: {0}")]
    Cache(String),
    /// A verified model bundle could not be decoded.
    #[error("model decoding failed: {0}")]
    Decode(String),
}

impl ModelsError {
    /// Returns whether the error identifies a missing scope, model, or model version.
    pub fn is_not_found(&self) -> bool {
        matches!(
            self,
            Self::ScopeNotFound | Self::ModelNotFound { .. } | Self::VersionNotFound { .. }
        )
    }

    /// Returns whether downloaded bytes failed integrity or path verification.
    pub fn is_verification(&self) -> bool {
        matches!(self, Self::Verification(_))
    }
}
