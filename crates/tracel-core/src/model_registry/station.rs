use tracel_artifact::download::ArtifactDownloadFile;

use crate::backend::station::StationBackend;
use crate::download_file::artifact_download_file_with_verification;
use crate::model_registry::{
    ModelInfo, ModelRegistryError, ModelRegistryProvider, ModelVersionInfo,
};

impl ModelRegistryProvider for StationBackend {
    fn get(&self, name: &str) -> Result<ModelInfo, ModelRegistryError> {
        let resp = self.client.models().get(name)?;
        Ok(ModelInfo {
            name: resp.name,
            description: resp.description,
            version_count: resp.version_count,
        })
    }

    fn version(&self, name: &str, version: u32) -> Result<ModelVersionInfo, ModelRegistryError> {
        let resp = self.client.models().version(name, version)?;
        Ok(ModelVersionInfo {
            version: resp.version,
            size: resp.size,
            checksum: resp.checksum,
        })
    }

    fn download_plan(
        &self,
        name: &str,
        version: u32,
    ) -> Result<Vec<ArtifactDownloadFile>, ModelRegistryError> {
        let resp = self.client.models().download(name, version)?;

        Ok(resp
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
            .collect())
    }
}
