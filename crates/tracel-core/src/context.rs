use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

#[cfg(feature = "station")]
use url::Url;

use crate::backend::cloud::CloudBackend;
use crate::backend::cloud::CloudError;
use crate::backend::local::LocalBackend;
#[cfg(feature = "station")]
use crate::backend::station::StationBackend;
use crate::experiment::Experiment;
use crate::experiment::ExperimentProvider;
use serde_json::Value;
use tracel_experiment::ExperimentRun;
use tracel_experiment::error::ExperimentError;

#[derive(Debug, Clone)]
pub struct Context {
    backend: Backend,
}

#[derive(Debug, Clone)]
pub enum Backend {
    Cloud(CloudBackend),
    Local(LocalBackend),
    #[cfg(feature = "station")]
    Station(StationBackend),
}

impl Context {
    pub(crate) fn new(backend: Backend) -> Self {
        Self { backend }
    }

    pub fn cloud() -> Result<Self, CloudError> {
        CloudBackend::create_context()
    }

    pub fn local(path: impl Into<PathBuf>) -> Self {
        LocalBackend::create_context(path)
    }

    #[cfg(feature = "station")]
    pub fn station(url: Url) -> Self {
        StationBackend::create_context(url)
    }

    pub fn experiment(&self) -> Experiment {
        Experiment::new(Arc::new(self.clone()))
    }
}

impl ExperimentProvider for Context {
    fn create_experiment(
        &self,
        routine: String,
        attributes: HashMap<String, Value>,
    ) -> Result<ExperimentRun, ExperimentError> {
        match &self.backend {
            Backend::Cloud(backend) => backend.create_experiment(routine, attributes),
            Backend::Local(backend) => backend.create_experiment(routine, attributes),
            #[cfg(feature = "station")]
            Backend::Station(backend) => backend.create_experiment(routine, attributes),
        }
    }
}
