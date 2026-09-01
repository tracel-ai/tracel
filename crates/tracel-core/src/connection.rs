use std::path::PathBuf;
use std::sync::Arc;

#[cfg(feature = "station")]
use url::Url;

use tracel_console::{Console, ConsoleError};
use tracel_experiment::ExperimentModule;
use tracel_inference::{InferenceModule, InferenceProvider};

use crate::backend::local::LocalBackend;
use crate::cloud::CloudError;
use crate::dataset::DatasetModule;
use crate::inference::DefaultInferenceProvider;
use crate::model_registry::ModelRegistryModule;
#[cfg(feature = "station")]
use tracel_station::Station;

/// The capabilities a connection makes available.
pub struct Capabilities {
    pub experiment: ExperimentModule,
    pub inference: InferenceModule,
    pub model_registry: Option<ModelRegistryModule>,
    pub dataset: Option<DatasetModule>,
}

#[derive(Debug, Clone)]
pub enum Connection {
    Cloud,
    Offline(PathBuf),
    #[cfg(feature = "station")]
    Station(Url),
}

impl Connection {
    pub(crate) fn into_capabilities(self) -> Result<Capabilities, ContextError> {
        match self {
            Connection::Cloud => {
                let env = crate::cloud::discover_env()?;
                let credentials = crate::cloud::discover_credentials(&env)?;
                let (namespace, project) = crate::cloud::discover_namespace_project()?;

                let console = Console::connect(env, &credentials)?;
                let project = console.project(namespace, project);

                Ok(Capabilities {
                    experiment: project.experiments(),
                    inference: project.inference(),
                    model_registry: None,
                    dataset: None,
                })
            }
            Connection::Offline(path) => {
                let backend = Arc::new(LocalBackend::create_context(path));
                Ok(Capabilities {
                    experiment: ExperimentModule::new(backend),
                    inference: default_inference(),
                    model_registry: None,
                    dataset: None,
                })
            }
            #[cfg(feature = "station")]
            Connection::Station(url) => {
                let station = Station::connect(url);
                Ok(Capabilities {
                    experiment: station.experiments(),
                    inference: default_inference(),
                    model_registry: None,
                    dataset: None,
                })
            }
        }
    }
}

/// Inference that ships nothing, for connections without an inference backend.
fn default_inference() -> InferenceModule {
    let provider: Arc<dyn InferenceProvider> = Arc::new(DefaultInferenceProvider::new());
    InferenceModule::new(provider)
}

#[derive(Debug, thiserror::Error)]
pub enum ContextError {
    #[error(transparent)]
    Cloud(#[from] CloudError),
    #[error(transparent)]
    Console(#[from] ConsoleError),
}
