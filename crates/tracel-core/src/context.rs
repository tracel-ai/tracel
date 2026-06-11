use std::path::PathBuf;
use std::sync::Arc;

#[cfg(feature = "station")]
use url::Url;

use crate::backend::cloud::CloudBackend;
use crate::backend::cloud::CloudError;
use crate::backend::local::LocalBackend;
#[cfg(feature = "station")]
use crate::backend::station::StationBackend;
use tracel_experiment::ExperimentModule;
use tracel_experiment::ExperimentProvider;

#[derive(Clone)]
pub struct Context {
    experiment_provider: Arc<dyn ExperimentProvider>,
}

impl Context {
    pub fn cloud() -> Result<Self, CloudError> {
        let backend = CloudBackend::create_context()?;
        Ok(Self {
            experiment_provider: Arc::new(backend),
        })
    }

    pub fn local(path: impl Into<PathBuf>) -> Self {
        let backend = LocalBackend::create_context(path);
        Self {
            experiment_provider: Arc::new(backend),
        }
    }

    #[cfg(feature = "station")]
    pub fn station(url: Url) -> Self {
        let backend = StationBackend::create_context(url);
        Self {
            experiment_provider: Arc::new(backend),
        }
    }

    pub fn experiment(&self) -> ExperimentModule {
        ExperimentModule::new(self.experiment_provider.clone())
    }
}
