use tracel_artifact::bundle::FsBundle;
use tracel_client::ClientError;

use crate::backend::station::StationBackend;
use crate::download_file::artifact_download_file_with_verification;
use crate::model_registry::{ModelRegistryError, ModelRegistryProvider, ensure_exists};

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
            .map(|f| {
                artifact_download_file_with_verification(
                    f.rel_path,
                    f.url,
                    f.size_bytes,
                    f.checksum,
                )
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
            return err.into();
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
        ensure_exists(self.client.models().get(name), || {
            ModelRegistryError::ModelNotFound {
                name: name.to_string(),
            }
        })
    }

    fn ensure_version_exists(&self, name: &str, version: u32) -> Result<(), ModelRegistryError> {
        ensure_exists(self.client.models().version(name, version), || {
            ModelRegistryError::VersionNotFound {
                name: name.to_string(),
                version,
            }
        })
    }
}
