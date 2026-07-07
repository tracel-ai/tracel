mod cache;
mod cloud;
#[cfg(feature = "station")]
mod station;

pub(crate) use cache::ModelCache;

use std::sync::Arc;

use tracel_artifact::bundle::{BundleDecode, FsBundle};
use tracel_artifact::download::{ArtifactDownloadFile, DownloadError};
use tracel_client::ClientError;

/// Plain data describing a model version, as fetched from a [`ModelRegistryProvider`] before
/// its files are downloaded.
#[derive(Debug, Clone)]
pub(crate) struct ModelInfo {
    pub files: Vec<ArtifactDownloadFile>,
}

#[derive(Debug, thiserror::Error)]
pub enum ModelRegistryError {
    #[error("model '{name}' not found")]
    ModelNotFound { name: String },
    #[error("version {version} of model '{name}' not found")]
    VersionNotFound { name: String, version: u32 },
    #[error("communication with the model registry failed: {0}")]
    Client(#[from] ClientError),
    #[error("failed to download model files: {0}")]
    Download(#[from] DownloadError),
    #[error("failed to decode downloaded model: {0}")]
    DecodeError(Box<dyn std::error::Error>),
}

pub trait ModelRegistryProvider: Send + Sync {
    /// TODO: docs
    fn load_model_bundle(&self, name: &str, version: u32) -> Result<FsBundle, ModelRegistryError>;
}

#[derive(Clone)]
pub struct ModelRegistryModule {
    provider: Arc<dyn ModelRegistryProvider>,
}

impl ModelRegistryModule {
    pub fn new(provider: Arc<dyn ModelRegistryProvider>) -> Self {
        Self { provider }
    }

    // TODO: docs
    pub fn load<D: BundleDecode>(
        &self,
        name: &str,
        version: u32,
        settings: &D::Settings,
    ) -> Result<D, ModelRegistryError> {
        let source = self.provider.load_model_bundle(name, version)?;
        D::decode(&source, settings).map_err(|e| ModelRegistryError::DecodeError(e.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use tracel_artifact::bundle::{BundleSink, BundleSource};

    struct FakeProvider<F> {
        load: F,
    }

    impl<F> ModelRegistryProvider for FakeProvider<F>
    where
        F: Fn(&str, u32) -> Result<FsBundle, ModelRegistryError> + Send + Sync,
    {
        fn load_model_bundle(
            &self,
            name: &str,
            version: u32,
        ) -> Result<FsBundle, ModelRegistryError> {
            (self.load)(name, version)
        }
    }

    #[derive(Debug, PartialEq)]
    struct TestArtifact {
        value: String,
    }

    impl BundleDecode for TestArtifact {
        type Settings = ();
        type Error = String;

        fn decode<I: BundleSource>(
            source: &I,
            _settings: &Self::Settings,
        ) -> Result<Self, Self::Error> {
            let mut reader = source.open("value.txt")?;
            let mut value = String::new();
            reader
                .read_to_string(&mut value)
                .map_err(|e| e.to_string())?;
            Ok(TestArtifact { value })
        }
    }

    fn bundle_with_value(value: &str) -> FsBundle {
        let mut bundle = FsBundle::temp().unwrap();
        bundle.put_bytes("value.txt", value.as_bytes()).unwrap();
        bundle
    }

    #[test]
    fn given_provider_returns_bundle_when_load_then_decodes_artifact() {
        let provider = FakeProvider {
            load: |_name: &str, _version: u32| Ok(bundle_with_value("hello")),
        };
        let module = ModelRegistryModule::new(Arc::new(provider));

        let artifact: TestArtifact = module.load("mnist", 1, &()).unwrap();

        assert_eq!(artifact.value, "hello");
    }

    #[test]
    fn given_provider_returns_model_not_found_when_load_then_error_is_propagated() {
        let provider = FakeProvider {
            load: |name: &str, _version: u32| {
                Err(ModelRegistryError::ModelNotFound {
                    name: name.to_string(),
                })
            },
        };
        let module = ModelRegistryModule::new(Arc::new(provider));

        let result: Result<TestArtifact, _> = module.load("mnist", 1, &());

        assert!(matches!(
            result,
            Err(ModelRegistryError::ModelNotFound { name }) if name == "mnist"
        ));
    }

    #[test]
    fn given_provider_returns_version_not_found_when_load_then_error_is_propagated() {
        let provider = FakeProvider {
            load: |name: &str, version: u32| {
                Err(ModelRegistryError::VersionNotFound {
                    name: name.to_string(),
                    version,
                })
            },
        };
        let module = ModelRegistryModule::new(Arc::new(provider));

        let result: Result<TestArtifact, _> = module.load("mnist", 1, &());

        assert!(matches!(
            result,
            Err(ModelRegistryError::VersionNotFound { name, version })
                if name == "mnist" && version == 1
        ));
    }

    #[test]
    fn given_provider_returns_client_error_when_load_then_error_is_propagated() {
        let provider = FakeProvider {
            load: |_name: &str, _version: u32| {
                Err(ModelRegistryError::Client(ClientError::NotFound))
            },
        };
        let module = ModelRegistryModule::new(Arc::new(provider));

        let result: Result<TestArtifact, _> = module.load("mnist", 1, &());

        assert!(matches!(
            result,
            Err(ModelRegistryError::Client(ClientError::NotFound))
        ));
    }

    #[test]
    fn given_provider_returns_download_error_when_load_then_error_is_propagated() {
        let provider = FakeProvider {
            load: |_name: &str, _version: u32| {
                Err(ModelRegistryError::Download(DownloadError::TargetError(
                    "boom".to_string(),
                )))
            },
        };
        let module = ModelRegistryModule::new(Arc::new(provider));

        let result: Result<TestArtifact, _> = module.load("mnist", 1, &());

        assert!(matches!(
            result,
            Err(ModelRegistryError::Download(DownloadError::TargetError(msg))) if msg == "boom"
        ));
    }

    #[test]
    fn given_bundle_missing_expected_file_when_load_then_returns_decode_error() {
        let provider = FakeProvider {
            load: |_name: &str, _version: u32| Ok(FsBundle::temp().unwrap()),
        };
        let module = ModelRegistryModule::new(Arc::new(provider));

        let result: Result<TestArtifact, _> = module.load("mnist", 1, &());

        assert!(matches!(result, Err(ModelRegistryError::DecodeError(_))));
    }
}
