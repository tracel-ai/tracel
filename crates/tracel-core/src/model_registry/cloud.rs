use tracel_artifact::download::ArtifactDownloadFile;
use tracel_client::ClientError;

use crate::backend::cloud::CloudBackend;
use crate::download_file::artifact_download_file;
use crate::model_registry::{ModelInfo, ModelRegistryError, ModelRegistryProvider};

impl ModelRegistryProvider for CloudBackend {
    fn get(&self, name: &str, version: u32) -> Result<ModelInfo, ModelRegistryError> {
        let resp_model = self
            .client
            .get_model(&self.namespace, &self.project, name)
            .map_err(|err| {
                map_not_found(err, || ModelRegistryError::ModelNotFound {
                    name: name.to_string(),
                })
            })?;
        let resp_version = self
            .client
            .get_model_version(&self.namespace, &self.project, name, version)
            .map_err(|err| {
                map_not_found(err, || ModelRegistryError::VersionNotFound {
                    name: name.to_string(),
                    version,
                })
            })?;

        Ok(ModelInfo {
            name: resp_model.name,
            description: resp_model.description,
            version_count: resp_model.version_count,
            version: resp_version.version,
            size: resp_version.size,
            checksum: resp_version.checksum,
        })
    }

    fn download_plan(
        &self,
        name: &str,
        version: u32,
    ) -> Result<Vec<ArtifactDownloadFile>, ModelRegistryError> {
        let resp =
            self.client
                .presign_model_download(&self.namespace, &self.project, name, version)?;

        Ok(resp
            .files
            .into_iter()
            .map(|f| artifact_download_file(f.rel_path, f.url))
            .collect())
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
