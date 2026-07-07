use tracel_artifact::bundle::FsBundle;

use crate::backend::cloud::CloudBackend;
use crate::download_file::artifact_download_file;
use crate::model_registry::{ModelRegistryError, ModelRegistryProvider, map_not_found};

impl ModelRegistryProvider for CloudBackend {
    fn load_model_bundle(&self, name: &str, version: u32) -> Result<FsBundle, ModelRegistryError> {
        self.ensure_model_exists(name)?;
        self.ensure_version_exists(name, version)?;

        let resp_download = self
            .client
            .presign_model_download(&self.namespace, &self.project, name, version)
            .map_err(|err| {
                map_not_found(err, || ModelRegistryError::VersionNotFound {
                    name: name.to_string(),
                    version,
                })
            })?;

        let files: Vec<_> = resp_download
            .files
            .into_iter()
            .map(|f| artifact_download_file(f.rel_path, f.url))
            .collect();

        self.model_cache
            .get_or_download(&self.file_transfer_client, name, version, &files)
    }
}

impl CloudBackend {
    fn ensure_model_exists(&self, name: &str) -> Result<(), ModelRegistryError> {
        self.client
            .get_model(&self.namespace, &self.project, name)
            .map(|_| ())
            .map_err(|err| {
                map_not_found(err, || ModelRegistryError::ModelNotFound {
                    name: name.to_string(),
                })
            })
    }

    fn ensure_version_exists(&self, name: &str, version: u32) -> Result<(), ModelRegistryError> {
        self.client
            .get_model_version(&self.namespace, &self.project, name, version)
            .map(|_| ())
            .map_err(|err| {
                map_not_found(err, || ModelRegistryError::VersionNotFound {
                    name: name.to_string(),
                    version,
                })
            })
    }
}
