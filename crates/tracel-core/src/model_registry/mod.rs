mod cache;
mod cloud;
#[cfg(feature = "station")]
mod station;

pub(crate) use cache::ModelCache;

use std::sync::Arc;

use tracel_artifact::bundle::{BundleDecode, FsBundle};
#[cfg(test)]
use tracel_artifact::download::DownloadError;
#[cfg(test)]
use tracel_client::ClientError;

type ModelLoader =
    dyn Fn(&str, u32) -> Result<FsBundle, ModelRegistryError> + Send + Sync + 'static;

#[derive(Debug, thiserror::Error)]
pub enum ModelRegistryError {
    #[error("model '{name}' not found")]
    ModelNotFound { name: String },
    #[error("version {version} of model '{name}' not found")]
    VersionNotFound { name: String, version: u32 },
    #[error("communication with the model registry failed: {0}")]
    Client(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("failed to download model files: {0}")]
    Download(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("failed to decode downloaded model: {0}")]
    DecodeError(Box<dyn std::error::Error>),
}

#[derive(Clone)]
pub struct ModelRegistryModule {
    load_model_bundle: Arc<ModelLoader>,
}

impl ModelRegistryModule {
    pub fn new(
        load_model_bundle: impl Fn(&str, u32) -> Result<FsBundle, ModelRegistryError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            load_model_bundle: Arc::new(load_model_bundle),
        }
    }

    /// Loads model `name` at `version`, decoding its downloaded bundle into `D` using `settings`.
    pub fn load<D: BundleDecode>(
        &self,
        name: &str,
        version: u32,
        settings: &D::Settings,
    ) -> Result<D, ModelRegistryError> {
        let source = (self.load_model_bundle)(name, version)?;
        D::decode(&source, settings).map_err(|e| ModelRegistryError::DecodeError(e.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use tracel_artifact::bundle::{BundleSink, BundleSource};

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
        let module =
            ModelRegistryModule::new(|_name: &str, _version: u32| Ok(bundle_with_value("hello")));

        let artifact: TestArtifact = module.load("mnist", 1, &()).unwrap();

        assert_eq!(artifact.value, "hello");
    }

    #[test]
    fn given_provider_returns_model_not_found_when_load_then_error_is_propagated() {
        let module = ModelRegistryModule::new(|name: &str, _version: u32| {
            Err(ModelRegistryError::ModelNotFound {
                name: name.to_string(),
            })
        });

        let result: Result<TestArtifact, _> = module.load("mnist", 1, &());

        assert!(matches!(
            result,
            Err(ModelRegistryError::ModelNotFound { name }) if name == "mnist"
        ));
    }

    #[test]
    fn given_provider_returns_version_not_found_when_load_then_error_is_propagated() {
        let module = ModelRegistryModule::new(|name: &str, version: u32| {
            Err(ModelRegistryError::VersionNotFound {
                name: name.to_string(),
                version,
            })
        });

        let result: Result<TestArtifact, _> = module.load("mnist", 1, &());

        assert!(matches!(
            result,
            Err(ModelRegistryError::VersionNotFound { name, version })
                if name == "mnist" && version == 1
        ));
    }

    #[test]
    fn given_provider_returns_client_error_when_load_then_error_is_propagated() {
        let module = ModelRegistryModule::new(|_name: &str, _version: u32| {
            Err(ModelRegistryError::Client(Box::new(ClientError::NotFound)))
        });

        let result: Result<TestArtifact, _> = module.load("mnist", 1, &());

        assert!(matches!(result, Err(ModelRegistryError::Client(_))));
    }

    #[test]
    fn given_provider_returns_download_error_when_load_then_error_is_propagated() {
        let module = ModelRegistryModule::new(|_name: &str, _version: u32| {
            Err(ModelRegistryError::Download(Box::new(
                DownloadError::TargetError("boom".to_string()),
            )))
        });

        let result: Result<TestArtifact, _> = module.load("mnist", 1, &());

        match result {
            Err(ModelRegistryError::Download(e)) => {
                let e = e
                    .downcast_ref::<DownloadError>()
                    .expect("expected DownloadError");
                assert!(matches!(e, DownloadError::TargetError(msg) if msg == "boom"));
            }
            other => panic!("expected Download error, got {other:?}"),
        }
    }

    #[test]
    fn given_bundle_missing_expected_file_when_load_then_returns_decode_error() {
        let module =
            ModelRegistryModule::new(|_name: &str, _version: u32| Ok(FsBundle::temp().unwrap()));

        let result: Result<TestArtifact, _> = module.load("mnist", 1, &());

        assert!(matches!(result, Err(ModelRegistryError::DecodeError(_))));
    }
}
