use std::sync::Arc;

use crate::connexion::{Connexion, ContextError};
use tracel_experiment::ExperimentModule;
use tracel_experiment::ExperimentProvider;

#[derive(Clone)]
pub struct Context {
    experiment_provider: Arc<dyn ExperimentProvider>,
}

impl Context {
    pub fn new(connexion: Connexion) -> Result<Self, ContextError> {
        Ok(Self {
            experiment_provider: connexion.into_provider()?,
        })
    }

    pub fn experiment(&self) -> ExperimentModule {
        ExperimentModule::new(self.experiment_provider.clone())
    }
}
