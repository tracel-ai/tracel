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

#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub name: String,
    pub description: Option<String>,
    pub version_count: u64,
}

#[derive(Debug, Clone)]
pub struct ModelVersionInfo {
    pub version: u32,
    pub size: u64,
    pub checksum: String,
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
    /// Fetch metadata about a model by name.
    fn get(&self, name: &str) -> Result<ModelInfo, ModelRegistryError>;
    /// Fetch metadata about a specific version of a model.
    fn version(&self, name: &str, version: u32) -> Result<ModelVersionInfo, ModelRegistryError>;
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
    transfer_client: ReqwestTransferClient,
}

impl ModelRegistryModule {
    pub fn new(provider: Arc<dyn ModelRegistryProvider>) -> Self {
        Self {
            provider,
            transfer_client: ReqwestTransferClient::new(),
        }
    }

    pub fn get(&self, name: &str) -> Result<ModelInfo, ModelRegistryError> {
        self.provider.get(name).map_err(|err| match err {
            ModelRegistryError::Client(ClientError::NotFound) => ModelRegistryError::ModelNotFound {
                name: name.to_string(),
            },
            other => other,
        })
    }

    pub fn version(
        &self,
        name: &str,
        version: u32,
    ) -> Result<ModelVersionInfo, ModelRegistryError> {
        self.provider.version(name, version).map_err(|err| match err {
            ModelRegistryError::Client(ClientError::NotFound) => ModelRegistryError::VersionNotFound {
                name: name.to_string(),
                version,
            },
            other => other,
        })
    }

    pub fn download_to(
        &self,
        name: &str,
        version: u32,
        sink: &mut impl BundleSink,
    ) -> Result<(), ModelRegistryError> {
        self.download_to_with_client(name, version, sink, &self.transfer_client)
    }

    // Method to test the download_to method with a mock client
    fn download_to_with_client<FTC: FileTransferClient>(
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::{Cursor, Read};
    use std::sync::Mutex;
    use tracel_artifact::TransferError;
    use tracel_artifact::bundle::InMemoryBundleSources;
    use tracel_client::ClientError;

    #[derive(Clone, Default)]
    struct MockTransferClient {
        files: HashMap<String, Vec<u8>>,
    }

    impl FileTransferClient for MockTransferClient {
        fn put_reader<R: Read + Send + 'static>(
            &self,
            _url: &str,
            _reader: R,
            _size_bytes: u64,
        ) -> Result<(), TransferError> {
            unimplemented!("uploads are not exercised by ModelRegistryModule")
        }

        fn get_reader(&self, url: &str) -> Result<Box<dyn Read + Send>, TransferError> {
            let bytes = self
                .files
                .get(url)
                .ok_or_else(|| TransferError::Transport(format!("missing url in mock: {url}")))?;
            Ok(Box::new(Cursor::new(bytes.clone())))
        }
    }

    #[derive(Default)]
    struct FakeModelRegistryProvider {
        get_result: Mutex<Option<Result<ModelInfo, ModelRegistryError>>>,
        version_result: Mutex<Option<Result<ModelVersionInfo, ModelRegistryError>>>,
        download_plan_result: Mutex<Option<Result<Vec<ArtifactDownloadFile>, ModelRegistryError>>>,
    }

    impl ModelRegistryProvider for FakeModelRegistryProvider {
        fn get(&self, _name: &str) -> Result<ModelInfo, ModelRegistryError> {
            self.get_result
                .lock()
                .unwrap()
                .take()
                .expect("unexpected call to get()")
        }

        fn version(
            &self,
            _name: &str,
            _version: u32,
        ) -> Result<ModelVersionInfo, ModelRegistryError> {
            self.version_result
                .lock()
                .unwrap()
                .take()
                .expect("unexpected call to version()")
        }

        fn download_plan(
            &self,
            _name: &str,
            _version: u32,
        ) -> Result<Vec<ArtifactDownloadFile>, ModelRegistryError> {
            self.download_plan_result
                .lock()
                .unwrap()
                .take()
                .expect("unexpected call to download_plan()")
        }
    }

    #[test]
    fn when_get_then_returns_model_info() {
        let provider = FakeModelRegistryProvider {
            get_result: Mutex::new(Some(Ok(ModelInfo {
                name: "resnet50".to_string(),
                description: Some("image classifier".to_string()),
                version_count: 3,
            }))),
            ..Default::default()
        };
        let module = ModelRegistryModule::new(Arc::new(provider));

        let info = module.get("resnet50").expect("get should succeed");

        assert_eq!(info.name, "resnet50");
        assert_eq!(info.version_count, 3);
    }

    #[test]
    fn given_provider_returns_error_when_get_then_propagates_the_error() {
        let provider = FakeModelRegistryProvider {
            get_result: Mutex::new(Some(Err(ClientError::NotFound.into()))),
            ..Default::default()
        };
        let module = ModelRegistryModule::new(Arc::new(provider));

        let err = module.get("missing-model").expect_err("get should fail");

        assert!(matches!(
            err,
            ModelRegistryError::ModelNotFound { name } if name == "missing-model"
        ));
    }

    #[test]
    fn when_version_then_returns_version_info() {
        let provider = FakeModelRegistryProvider {
            version_result: Mutex::new(Some(Ok(ModelVersionInfo {
                version: 2,
                size: 1024,
                checksum: "abc123".to_string(),
            }))),
            ..Default::default()
        };
        let module = ModelRegistryModule::new(Arc::new(provider));

        let info = module
            .version("resnet50", 2)
            .expect("version should succeed");

        assert_eq!(info.version, 2);
        assert_eq!(info.size, 1024);
    }

    #[test]
    fn given_provider_returns_an_error_when_version_then_propagates_the_error() {
        let provider = FakeModelRegistryProvider {
            version_result: Mutex::new(Some(Err(ClientError::NotFound.into()))),
            ..Default::default()
        };
        let module = ModelRegistryModule::new(Arc::new(provider));

        let err = module
            .version("resnet50", 99)
            .expect_err("version should fail");

        assert!(matches!(
            err,
            ModelRegistryError::VersionNotFound { name, version }
                if name == "resnet50" && version == 99
        ));
    }

    #[test]
    fn given_download_plan_fails_when_download_to_then_propagates_error() {
        let provider = FakeModelRegistryProvider {
            download_plan_result: Mutex::new(Some(Err(ClientError::NotFound.into()))),
            ..Default::default()
        };
        let module = ModelRegistryModule::new(Arc::new(provider));
        let mut sink = InMemoryBundleSources::new();

        let err = module
            .download_to("resnet50", 2, &mut sink)
            .expect_err("download_to should fail");

        assert!(matches!(
            err,
            ModelRegistryError::VersionNotFound { name, version }
                if name == "resnet50" && version == 2
        ));
        assert_eq!(sink.len(), 0);
    }

    #[test]
    fn given_an_empty_download_plan_when_download_to_then_succeeds() {
        let provider = FakeModelRegistryProvider {
            download_plan_result: Mutex::new(Some(Ok(vec![]))),
            ..Default::default()
        };
        let module = ModelRegistryModule::new(Arc::new(provider));
        let mut sink = InMemoryBundleSources::new();

        module
            .download_to("resnet50", 2, &mut sink)
            .expect("download_to should succeed with no files to fetch");

        assert_eq!(sink.len(), 0);
    }

    #[test]
    fn given_a_download_plan_with_files_when_download_to_then_writes_files_into_sink() {
        let data = b"model weights".to_vec();
        let provider = FakeModelRegistryProvider {
            download_plan_result: Mutex::new(Some(Ok(vec![ArtifactDownloadFile {
                rel_path: "weights.bin".to_string(),
                url: "mock://weights".to_string(),
                size_bytes: None,
                checksum: None,
            }]))),
            ..Default::default()
        };
        let transfer_client = MockTransferClient {
            files: HashMap::from([("mock://weights".to_string(), data.clone())]),
        };
        let module = ModelRegistryModule::new(Arc::new(provider));
        let mut sink = InMemoryBundleSources::new();

        module
            .download_to_with_client("resnet50", 2, &mut sink, &transfer_client)
            .expect("download_to should succeed");

        assert_eq!(sink.len(), 1);
        assert_eq!(sink.files()[0].dest_path(), "weights.bin");
        assert_eq!(sink.files()[0].source(), data);
    }

    #[test]
    fn given_transfer_client_cannot_find_the_file_when_download_to_then_model_registry_error_is_thrown()
     {
        let provider = FakeModelRegistryProvider {
            download_plan_result: Mutex::new(Some(Ok(vec![ArtifactDownloadFile {
                rel_path: "weights.bin".to_string(),
                url: "mock://missing".to_string(),
                size_bytes: None,
                checksum: None,
            }]))),
            ..Default::default()
        };
        let transfer_client = MockTransferClient::default();
        let module = ModelRegistryModule::new(Arc::new(provider));
        let mut sink = InMemoryBundleSources::new();

        let err = module
            .download_to_with_client("resnet50", 2, &mut sink, &transfer_client)
            .expect_err("download_to should fail when the file can't be fetched");

        assert!(matches!(err, ModelRegistryError::Download(_)));
        assert_eq!(sink.len(), 0);
    }
}
