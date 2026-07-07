use tracel_artifact::bundle::FsBundle;
use tracel_artifact::download::download_artifacts_to_sink_with_client;
use tracel_client::ClientError;

use crate::backend::station::StationBackend;
use crate::download_file::artifact_download_file_with_verification;
use crate::model_registry::{ModelInfo, ModelRegistryError, ModelRegistryProvider};

impl ModelRegistryProvider for StationBackend {
    fn load_model_bundle(&self, name: &str, version: u32) -> Result<FsBundle, ModelRegistryError> {
        self.ensure_model_exists(name)?;
        self.ensure_version_exists(name, version)?;

        let resp_download = self
            .client
            .models()
            .download(name, version)
            .map_err(|err| {
                map_not_found(err, || ModelRegistryError::VersionNotFound {
                    name: name.to_string(),
                    version,
                })
            })?;

        let info = ModelInfo {
            files: resp_download
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
                .collect(),
        };

        if let Some(cached) = self.model_cache.get(name, version, &info.files) {
            return Ok(cached);
        }

        let mut bundle = self.model_cache.reserve(name, version).map_err(|e| {
            ModelRegistryError::Download(tracel_artifact::download::DownloadError::TargetError(
                e.to_string(),
            ))
        })?;
        download_artifacts_to_sink_with_client(
            &self.file_transfer_client,
            &mut bundle,
            &info.files,
        )?;

        Ok(bundle)
    }
}

impl StationBackend {
    fn ensure_model_exists(&self, name: &str) -> Result<(), ModelRegistryError> {
        self.client.models().get(name).map(|_| ()).map_err(|err| {
            map_not_found(err, || ModelRegistryError::ModelNotFound {
                name: name.to_string(),
            })
        })
    }

    fn ensure_version_exists(&self, name: &str, version: u32) -> Result<(), ModelRegistryError> {
        self.client
            .models()
            .version(name, version)
            .map(|_| ())
            .map_err(|err| {
                map_not_found(err, || ModelRegistryError::VersionNotFound {
                    name: name.to_string(),
                    version,
                })
            })
    }
}

fn map_not_found(
    err: ClientError,
    not_found: impl FnOnce() -> ModelRegistryError,
) -> ModelRegistryError {
    match err {
        ClientError::NotFound => not_found(),
        other => other.into(),
    }
}
