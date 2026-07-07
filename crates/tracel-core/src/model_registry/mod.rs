mod cache;
mod cloud;
#[cfg(feature = "station")]
mod station;

pub(crate) use cache::ModelCache;

use std::sync::Arc;

use tracel_artifact::bundle::{BundleDecode, FsBundle};
use tracel_artifact::download::{ArtifactDownloadFile, DownloadError};
use tracel_client::ClientError;

/// Plain data describing a model version, as fetched from a [`ModelRegistryProvider`] before
/// its files are downloaded.
#[derive(Debug, Clone)]
pub(crate) struct ModelInfo {
    pub files: Vec<ArtifactDownloadFile>,
}

#[derive(Debug, thiserror::Error)]
pub enum ModelRegistryError {
    #[error("model '{name}' not found")]
    ModelNotFound { name: String },
    #[error("version {version} of model '{name}' not found")]
    VersionNotFound { name: String, version: u32 },
    #[error("communication with the model registry failed: {0}")]
    Client(#[from] ClientError),
    #[error("failed to download model files: {0}")]
    Download(#[from] DownloadError),
    #[error("failed to decode downloaded model: {0}")]
    DecodeError(Box<dyn std::error::Error>),
}

pub trait ModelRegistryProvider: Send + Sync {
    /// TODO: docs
    fn load_model_bundle(&self, name: &str, version: u32) -> Result<FsBundle, ModelRegistryError>;
}

#[derive(Clone)]
pub struct ModelRegistryModule {
    provider: Arc<dyn ModelRegistryProvider>,
}

impl ModelRegistryModule {
    pub fn new(provider: Arc<dyn ModelRegistryProvider>) -> Self {
        Self { provider }
    }

    // TODO: docs
    pub fn load<D: BundleDecode>(
        &self,
        name: &str,
        version: u32,
        settings: &D::Settings,
    ) -> Result<D, ModelRegistryError> {
        let source = self.provider.load_model_bundle(name, version)?;
        D::decode(&source, settings).map_err(|e| ModelRegistryError::DecodeError(e.into()))
    }
}
