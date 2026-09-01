use std::sync::Arc;

use tracel_datasets::Datasets;
use tracel_experiment::ExperimentModule;
use tracel_inference::InferenceModule;
use tracel_models::Models;

use crate::backend::Backend;
use crate::connection::{Connection, ContextError};

#[derive(Clone)]
pub struct Context {
    backend: Arc<dyn Backend>,
}

impl Context {
    pub fn new(connection: Connection) -> Result<Self, ContextError> {
        Ok(Self {
            backend: connection.into_backend()?,
        })
    }

    pub fn experiment(&self) -> ExperimentModule {
        self.backend.experiments()
    }

    pub fn inference(&self) -> InferenceModule {
        self.backend.inference()
    }

    pub fn models(&self) -> Option<Models> {
        self.backend.models()
    }

    pub fn datasets(&self) -> Option<Datasets> {
        self.backend.datasets()
    }
}
