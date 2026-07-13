use std::path::PathBuf;
use std::sync::Arc;

#[cfg(feature = "station")]
use url::Url;

use crate::backend::cloud::{CloudBackend, CloudError};
use crate::backend::local::LocalBackend;
#[cfg(feature = "station")]
use crate::backend::station::{StationBackend, StationError};
use crate::inference::{CloudInferenceProvider, DefaultInferenceProvider};
use crate::model_registry::ModelRegistryProvider;
use tracel_experiment::ExperimentProvider;
use tracel_inference::InferenceProvider;

pub struct Providers {
    pub experiment: Arc<dyn ExperimentProvider>,
    pub inference: Arc<dyn InferenceProvider>,
    pub model_registry: Option<Arc<dyn ModelRegistryProvider>>,
}

#[derive(Debug, Clone)]
pub enum Connection {
    Cloud,
    Offline(PathBuf),
    #[cfg(feature = "station")]
    Station(Url),
}

impl Connection {
    pub(crate) fn into_providers(self) -> Result<Providers, ContextError> {
        match self {
            Connection::Cloud => {
                let backend = Arc::new(CloudBackend::create_context()?);
                let inference = Arc::new(CloudInferenceProvider::new(
                    backend.client.clone(),
                    backend.namespace.clone(),
                    backend.project.clone(),
                ));
                Ok(Providers {
                    experiment: backend.clone(),
                    inference,
                    model_registry: Some(backend),
                })
            }
            Connection::Offline(path) => {
                let backend = Arc::new(LocalBackend::create_context(path));
                Ok(Providers {
                    experiment: backend,
                    inference: Arc::new(DefaultInferenceProvider::new()),
                    model_registry: None,
                })
            }
            #[cfg(feature = "station")]
            Connection::Station(url) => {
                let backend = Arc::new(StationBackend::create_context(url)?);
                Ok(Providers {
                    experiment: backend.clone(),
                    inference: Arc::new(DefaultInferenceProvider::new()),
                    model_registry: Some(backend),
                })
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ContextError {
    #[error(transparent)]
    Cloud(#[from] CloudError),
    #[cfg(feature = "station")]
    #[error(transparent)]
    Station(#[from] StationError),
}
