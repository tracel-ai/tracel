use std::sync::Arc;

use crate::connection::{Connection, ContextError};
use crate::model_registry::{ModelRegistryModule, ModelRegistryProvider};
use tracel_experiment::ExperimentModule;
use tracel_experiment::ExperimentProvider;

#[derive(Clone)]
pub struct Context {
    experiment_provider: Arc<dyn ExperimentProvider>,
    model_registry_provider: Option<Arc<dyn ModelRegistryProvider>>,
}

impl Context {
    pub fn new(connection: Connection) -> Result<Self, ContextError> {
        let providers = connection.into_providers()?;
        Ok(Self {
            experiment_provider: providers.experiment,
            model_registry_provider: providers.model_registry,
        })
    }

    pub fn experiment(&self) -> ExperimentModule {
        ExperimentModule::new(self.experiment_provider.clone())
    }

    pub fn models(&self) -> Option<ModelRegistryModule> {
        self.model_registry_provider
            .clone()
            .map(ModelRegistryModule::new)
    }
}
