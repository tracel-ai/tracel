use std::sync::Arc;

use tracel_datasets::Datasets;
use tracel_experiment::ExperimentModule;
use tracel_inference::InferenceModule;
use tracel_models::Models;

use tracel_task::Job;

use crate::backend::Backend;
use crate::connection::{Connection, ContextError};

/// One connection's capabilities, shared across a program.
#[derive(Clone)]
pub struct Context {
    backend: Arc<dyn Backend>,
}

impl Context {
    /// Opens `connection`; the console is reached when the job is driven.
    pub fn new(connection: Connection) -> Job<Self, ContextError> {
        let backend = connection.into_backend();
        Job::new(async move {
            Ok(Self {
                backend: backend.await?,
            })
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
