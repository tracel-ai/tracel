mod cloud;
#[cfg(feature = "station")]
mod station;

use std::sync::Arc;

use tracel_artifact::bundle::BundleSink;
use tracel_artifact::download::{
    ArtifactDownloadFile, DownloadError, download_artifacts_to_sink_with_client,
};
use tracel_artifact::{FileTransferClient, ReqwestTransferClient};
use tracel_client::ClientError;

/// Plain data describing a model version, as fetched from a [`ModelRegistryProvider`].
pub struct ModelMetadata {
    pub name: String,
    pub description: Option<String>,
    pub version_count: u64,
    pub version: u32,
    pub size: u64,
    pub checksum: String,
}

/// Metadata about a model version, carrying the transfer client needed to download it.
#[derive(Clone)]
pub struct ModelInfo {
    pub name: String,
    pub description: Option<String>,
    pub version_count: u64,
    pub version: u32,
    pub size: u64,
    pub checksum: String,
    client: Arc<dyn FileTransferClient>,
}

impl std::fmt::Debug for ModelInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelInfo")
            .field("name", &self.name)
            .field("description", &self.description)
            .field("version_count", &self.version_count)
            .field("version", &self.version)
            .field("size", &self.size)
            .field("checksum", &self.checksum)
            .finish()
    }
}

impl ModelInfo {
    fn new(metadata: ModelMetadata, client: Arc<dyn FileTransferClient>) -> Self {
        Self {
            name: metadata.name,
            description: metadata.description,
            version_count: metadata.version_count,
            version: metadata.version,
            size: metadata.size,
            checksum: metadata.checksum,
            client,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ModelRegistryError {
    #[error("model '{name}' not found")]
    ModelNotFound { name: String },
    #[error("version {version} of model '{name}' not found")]
    VersionNotFound { name: String, version: u32 },
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error(transparent)]
    Download(#[from] DownloadError),
}

pub trait ModelRegistryProvider: Send + Sync {
    /// Fetch metadata about a specific version of a model by name.
    fn get(&self, name: &str, version: u32) -> Result<ModelMetadata, ModelRegistryError>;
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
    transfer_client: Arc<dyn FileTransferClient>,
}

impl ModelRegistryModule {
    pub fn new(provider: Arc<dyn ModelRegistryProvider>) -> Self {
        Self {
            provider,
            transfer_client: Arc::new(ReqwestTransferClient::new()),
        }
    }

    pub fn get(&self, name: &str, version: u32) -> Result<ModelInfo, ModelRegistryError> {
        let metadata = self.provider.get(name, version)?;
        Ok(ModelInfo::new(metadata, self.transfer_client.clone()))
    }

    pub fn download_to(
        &self,
        name: &str,
        version: u32,
        sink: &mut impl BundleSink,
    ) -> Result<(), ModelRegistryError> {
        self.download_to_with_client(name, version, sink, self.transfer_client.as_ref())
    }

    // Internal helper used by download_to; allows injecting a custom transfer client in tests.
    fn download_to_with_client<FTC: FileTransferClient + ?Sized>(
        &self,
        name: &str,
        version: u32,
        sink: &mut impl BundleSink,
        client: &FTC,
    ) -> Result<(), ModelRegistryError> {
        let files = self
            .provider
            .download_plan(name, version)
            .map_err(|err| match err {
                ModelRegistryError::Client(ClientError::NotFound) => {
                    ModelRegistryError::VersionNotFound {
                        name: name.to_string(),
                        version,
                    }
                }
                other => other,
            })?;
        download_artifacts_to_sink_with_client(client, sink, &files)?;
        Ok(())
    }
}
