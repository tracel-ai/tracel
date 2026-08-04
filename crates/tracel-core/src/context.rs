use std::sync::Arc;

use crate::backend::cloud::CloudError;
#[cfg(feature = "station")]
use crate::backend::station::StationError;
use crate::dataset::{DatasetModule, DatasetProvider};
use crate::model_registry::{ModelRegistryModule, ModelRegistryProvider};
use tracel_experiment::ExperimentModule;
use tracel_experiment::ExperimentProvider;
use tracel_inference::{InferenceModule, InferenceProvider};

#[derive(Debug, thiserror::Error)]
pub enum ContextError {
    #[error(transparent)]
    Cloud(#[from] CloudError),
    #[cfg(feature = "station")]
    #[error(transparent)]
    Station(#[from] StationError),
}

#[derive(Clone)]
pub struct Context {
    experiment_provider: Arc<dyn ExperimentProvider>,
    inference_provider: Arc<dyn InferenceProvider>,
    model_registry_provider: Option<Arc<dyn ModelRegistryProvider>>,
    dataset_provider: Option<Arc<dyn DatasetProvider>>,
}

pub struct Providers {
    pub experiment: Arc<dyn ExperimentProvider>,
    pub inference: Arc<dyn InferenceProvider>,
    pub model_registry: Option<Arc<dyn ModelRegistryProvider>>,
    pub dataset: Option<Arc<dyn DatasetProvider>>,
}

pub trait IntoProviders {
    fn into_providers(self) -> Result<Providers, ContextError>;
}

impl<T: IntoProviders + 'static> From<T> for Box<dyn IntoProviders> {
    fn from(backend: T) -> Self {
        Box::new(backend)
    }
}

impl Context {
    pub fn new(connection: impl IntoProviders) -> Result<Self, ContextError> {
        let providers = connection.into_providers()?;
        Ok(Self {
            experiment_provider: providers.experiment,
            inference_provider: providers.inference,
            model_registry_provider: providers.model_registry,
            dataset_provider: providers.dataset,
        })
    }

    pub fn experiment(&self) -> ExperimentModule {
        ExperimentModule::new(self.experiment_provider.clone())
    }

    pub fn inference(&self) -> InferenceModule {
        InferenceModule::new(self.inference_provider.clone())
    }

    pub fn models(&self) -> Option<ModelRegistryModule> {
        self.model_registry_provider
            .clone()
            .map(ModelRegistryModule::new)
    }

    pub fn datasets(&self) -> Option<DatasetModule> {
        self.dataset_provider.clone().map(DatasetModule::new)
    }
}
