use std::collections::BTreeMap;

use crate::api::{Client, ClientError};
use crate::artifacts::{ArtifactDecode, MemoryArtifactReader};
use crate::schemas::ModelPath;

/// A registry-like interface for downloading models from Burn Central.
/// This reuses the existing artifact infrastructure since models are essentially
/// bundles of files like artifacts.
#[derive(Clone)]
pub struct ModelRegistry {
    client: Client,
}

impl ModelRegistry {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    /// Create a scope for a specific model within a project.
    pub fn model(&self, model_path: ModelPath) -> ModelScope {
        ModelScope::new(self.client.clone(), model_path)
    }

    /// Download a specific model version and decode it using the ArtifactDecode trait.
    pub fn download<T: ArtifactDecode>(
        &self,
        model_path: ModelPath,
        version: u32,
        settings: &T::Settings,
    ) -> Result<T, ModelDownloadError> {
        let scope = self.model(model_path);
        scope.download(version, settings)
    }

    /// Download a specific model version as a memory reader for dynamic access.
    pub fn download_raw(
        &self,
        model_path: ModelPath,
        version: u32,
    ) -> Result<MemoryArtifactReader, ModelDownloadError> {
        let scope = self.model(model_path);
        scope.download_raw(version)
    }
}

/// A scope for operations on a specific model within a project.
#[derive(Clone)]
pub struct ModelScope {
    client: Client,
    model_path: ModelPath,
}

impl ModelScope {
    pub(crate) fn new(client: Client, model_path: ModelPath) -> Self {
        Self { client, model_path }
    }

    /// Download a specific version of this model and decode it using the ArtifactDecode trait.
    /// This allows reusing existing artifact decoders for models.
    pub fn download<T: ArtifactDecode>(
        &self,
        version: u32,
        settings: &T::Settings,
    ) -> Result<T, ModelDownloadError> {
        let reader = self.download_raw(version)?;
        T::decode(&reader, settings).map_err(|e| ModelDownloadError::Decode(e.into()))
    }

    /// Download a specific version of this model as a memory reader for dynamic access.
    pub fn download_raw(&self, version: u32) -> Result<MemoryArtifactReader, ModelDownloadError> {
        let resp = self.client.presign_model_download(
            self.model_path.namespace(),
            self.model_path.project_name(),
            self.model_path.model_name(),
            version,
        )?;

        let mut data = BTreeMap::new();

        for file in resp.files {
            let bytes = self.client.download_bytes_from_url(&file.url)?;
            data.insert(file.rel_path, bytes);
        }

        Ok(MemoryArtifactReader::new(data))
    }

    /// Get the model path this scope operates on.
    pub fn path(&self) -> &ModelPath {
        &self.model_path
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ModelDownloadError {
    #[error("Client error: {0}")]
    Client(#[from] ClientError),
    #[error("Decode error: {0}")]
    Decode(Box<dyn std::error::Error + Send + Sync + 'static>),
}
