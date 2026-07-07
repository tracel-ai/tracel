use tracel_artifact::bundle::FsBundle;
use tracel_artifact::download::download_artifacts_to_sink_with_client;
use tracel_client::ClientError;

use crate::backend::station::StationBackend;
use crate::download_file::artifact_download_file_with_verification;
use crate::model_registry::{ModelInfo, ModelRegistryError, ModelRegistryProvider};

impl ModelRegistryProvider for StationBackend {
    fn load_model_bundle(&self, name: &str, version: u32) -> Result<FsBundle, ModelRegistryError> {
        let resp_model = self.client.models().get(name).map_err(|err| {
            map_not_found(err, || ModelRegistryError::ModelNotFound {
                name: name.to_string(),
            })
        })?;
        let resp_version = self.client.models().version(name, version).map_err(|err| {
            map_not_found(err, || ModelRegistryError::VersionNotFound {
                name: name.to_string(),
                version,
            })
        })?;
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
            name: resp_model.name,
            description: resp_model.description,
            version_count: resp_model.version_count,
            version: resp_version.version,
            size: resp_version.size,
            checksum: resp_version.checksum,
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

        let mut bundle = FsBundle::temp().map_err(|e| {
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

fn map_not_found(
    err: ClientError,
    not_found: impl FnOnce() -> ModelRegistryError,
) -> ModelRegistryError {
    match err {
        ClientError::NotFound => not_found(),
        other => other.into(),
    }
}
