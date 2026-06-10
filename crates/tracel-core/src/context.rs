use std::path::PathBuf;
use std::sync::Arc;

#[cfg(feature = "station")]
use url::Url;

use crate::backend::cloud::CloudBackend;
use crate::backend::cloud::CloudError;
use crate::backend::local::LocalBackend;
#[cfg(feature = "station")]
use crate::backend::station::StationBackend;
use tracel_experiment::ExperimentClient;
use tracel_experiment::ExperimentProvider;

#[derive(Clone)]
pub struct Context {
    experiment_provider: Arc<dyn ExperimentProvider>,
}

impl Context {
    fn new(experiment_provider: impl ExperimentProvider) -> Self {
        Self {
            experiment_provider: Arc::new(experiment_provider),
        }
    }

    pub fn cloud() -> Result<Self, CloudError> {
        let backend = CloudBackend::create_context()?;
        Ok(Context::new(backend))
    }

    pub fn local(path: impl Into<PathBuf>) -> Self {
        let backend = LocalBackend::create_context(path);
        Context::new(backend)
    }

    #[cfg(feature = "station")]
    pub fn station(url: Url) -> Self {
        let backend = StationBackend::create_context(url);
        Context::new(backend)
    }

    pub fn experiment(&self) -> ExperimentClient {
        ExperimentClient::new(self.experiment_provider.clone())
    }
}
