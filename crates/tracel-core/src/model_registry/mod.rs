mod cloud;
#[cfg(feature = "station")]
mod station;

use std::sync::Arc;

use tracel_artifact::bundle::BundleSink;
use tracel_artifact::download::{ArtifactDownloadFile, DownloadError, download_artifacts_to_sink};
use tracel_client::ClientError;

/// Basic information about a registered model.
#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub name: String,
    pub description: Option<String>,
    pub version_count: u64,
}

#[derive(Debug, Clone)]
pub struct ModelVersionInfo {
    pub version: u32,
    pub size: u64,
    pub checksum: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ModelRegistryError {
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error(transparent)]
    Download(#[from] DownloadError),
}

pub trait ModelRegistryProvider: Send + Sync {
    /// Fetch metadata about a model by name.
    fn get(&self, name: &str) -> Result<ModelInfo, ModelRegistryError>;

    /// Fetch metadata about a specific version of a model.
    fn version(&self, name: &str, version: u32) -> Result<ModelVersionInfo, ModelRegistryError>;

    /// Build the list of files (with download URLs) needed to fetch a model version.
    fn download_plan(
        &self,
        name: &str,
        version: u32,
    ) -> Result<Vec<ArtifactDownloadFile>, ModelRegistryError>;
}

#[derive(Clone)]
pub struct ModelRegistryModule {
    provider: Arc<dyn ModelRegistryProvider>,
}

impl ModelRegistryModule {
    pub fn new(provider: Arc<dyn ModelRegistryProvider>) -> Self {
        Self { provider }
    }

    pub fn get(&self, name: &str) -> Result<ModelInfo, ModelRegistryError> {
        self.provider.get(name)
    }

    pub fn version(
        &self,
        name: &str,
        version: u32,
    ) -> Result<ModelVersionInfo, ModelRegistryError> {
        self.provider.version(name, version)
    }

    pub fn download_to(
        &self,
        name: &str,
        version: u32,
        sink: &mut impl BundleSink,
    ) -> Result<(), ModelRegistryError> {
        let files = self.provider.download_plan(name, version)?;
        download_artifacts_to_sink(sink, &files)?;
        Ok(())
    }
}
