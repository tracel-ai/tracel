use tracel_artifact::bundle::FsBundle;
use tracel_artifact::download::ArtifactDownloadFile;
use tracel_client::ClientError;

use crate::backend::station::StationBackend;
use crate::model_registry::{ModelRegistryError, ModelRegistryProvider};

impl ModelRegistryProvider for StationBackend {
    fn load_model_bundle(&self, name: &str, version: u32) -> Result<FsBundle, ModelRegistryError> {
        let resp_download = self
            .client
            .models()
            .download(name, version)
            .map_err(|err| self.describe_download_error(err, name, version))?;

        let files: Vec<_> = resp_download
            .files
            .into_iter()
            .map(|f| ArtifactDownloadFile {
                rel_path: f.rel_path,
                url: f.url,
                size_bytes: Some(f.size_bytes),
                checksum: Some(f.checksum),
            })
            .collect();

        self.model_cache
            .get_or_download(&self.file_transfer_client, name, version, &files)
    }
}

impl StationBackend {
    /// Turns a failed download-plan request into a precise not-found error. Only queries
    /// the model and version individually when the request actually failed as not-found,
    /// so a successful load (including a cache hit) pays for a single round trip instead
    /// of three.
    fn describe_download_error(
        &self,
        err: ClientError,
        name: &str,
        version: u32,
    ) -> ModelRegistryError {
        if !matches!(err, ClientError::NotFound) {
            return ModelRegistryError::Client(Box::new(err));
        }
        if let Err(e) = self.ensure_model_exists(name) {
            return e;
        }
        self.ensure_version_exists(name, version).err().unwrap_or(
            ModelRegistryError::VersionNotFound {
                name: name.to_string(),
                version,
            },
        )
    }

    fn ensure_model_exists(&self, name: &str) -> Result<(), ModelRegistryError> {
        match self.client.models().get(name) {
            Ok(_) => Ok(()),
            Err(ClientError::NotFound) => Err(ModelRegistryError::ModelNotFound {
                name: name.to_string(),
            }),
            Err(err) => Err(ModelRegistryError::Client(Box::new(err))),
        }
    }

    fn ensure_version_exists(&self, name: &str, version: u32) -> Result<(), ModelRegistryError> {
        match self.client.models().version(name, version) {
            Ok(_) => Ok(()),
            Err(ClientError::NotFound) => Err(ModelRegistryError::VersionNotFound {
                name: name.to_string(),
                version,
            }),
            Err(err) => Err(ModelRegistryError::Client(Box::new(err))),
        }
    }
}
