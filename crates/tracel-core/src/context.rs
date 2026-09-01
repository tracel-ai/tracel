use crate::connection::{Connection, ContextError};
use crate::dataset::DatasetModule;
use crate::model_registry::ModelRegistryModule;
use tracel_experiment::ExperimentModule;
use tracel_inference::InferenceModule;

#[derive(Clone)]
pub struct Context {
    experiment: ExperimentModule,
    inference: InferenceModule,
    model_registry: Option<ModelRegistryModule>,
    dataset: Option<DatasetModule>,
}

impl Context {
    pub fn new(connection: Connection) -> Result<Self, ContextError> {
        let capabilities = connection.into_capabilities()?;
        Ok(Self {
            experiment: capabilities.experiment,
            inference: capabilities.inference,
            model_registry: capabilities.model_registry,
            dataset: capabilities.dataset,
        })
    }

    pub fn experiment(&self) -> ExperimentModule {
        self.experiment.clone()
    }

    pub fn inference(&self) -> InferenceModule {
        self.inference.clone()
    }

    pub fn models(&self) -> Option<ModelRegistryModule> {
        self.model_registry.clone()
    }

    pub fn datasets(&self) -> Option<DatasetModule> {
        self.dataset.clone()
    }
}
