use tracel_artifact::download::ArtifactDownloadFile;

use crate::backend::cloud::CloudBackend;
use crate::download_file::artifact_download_file;
use crate::model_registry::{
    ModelInfo, ModelRegistryError, ModelRegistryProvider, ModelVersionInfo,
};

impl ModelRegistryProvider for CloudBackend {
    fn get(&self, name: &str) -> Result<ModelInfo, ModelRegistryError> {
        let resp = self
            .client
            .get_model(&self.namespace, &self.project, name)?;
        Ok(ModelInfo {
            name: resp.name,
            description: resp.description,
            version_count: resp.version_count,
        })
    }

    fn version(&self, name: &str, version: u32) -> Result<ModelVersionInfo, ModelRegistryError> {
        let resp = self
            .client
            .get_model_version(&self.namespace, &self.project, name, version)?;
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
