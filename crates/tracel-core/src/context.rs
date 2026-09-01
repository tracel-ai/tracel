use crate::connection::{Connection, ContextError};
use tracel_experiment::ExperimentModule;
use tracel_inference::InferenceModule;

#[derive(Clone)]
pub struct Context {
    experiment: ExperimentModule,
    inference: InferenceModule,
}

impl Context {
    pub fn new(connection: Connection) -> Result<Self, ContextError> {
        let capabilities = connection.into_capabilities()?;
        Ok(Self {
            experiment: capabilities.experiment,
            inference: capabilities.inference,
        })
    }

    pub fn experiment(&self) -> ExperimentModule {
        self.experiment.clone()
    }

    pub fn inference(&self) -> InferenceModule {
        self.inference.clone()
    }
}
