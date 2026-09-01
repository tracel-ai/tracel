pub(crate) mod local;

use std::sync::Arc;

use tracel_datasets::Datasets;
use tracel_experiment::ExperimentModule;
use tracel_inference::{InferenceModule, InferenceProvider};
use tracel_models::Models;

use crate::inference::DefaultInferenceProvider;

/// What a connection can vend.
///
/// A backend that does not serve models or datasets says so by leaving those absent.
pub trait Backend: Send + Sync + 'static {
    fn experiments(&self) -> ExperimentModule;

    fn inference(&self) -> InferenceModule {
        let provider: Arc<dyn InferenceProvider> = Arc::new(DefaultInferenceProvider::new());
        InferenceModule::new(provider)
    }

    fn models(&self) -> Option<Models> {
        None
    }

    fn datasets(&self) -> Option<Datasets> {
        None
    }
}

impl Backend for tracel_console::ProjectHandle {
    fn experiments(&self) -> ExperimentModule {
        tracel_console::ProjectHandle::experiments(self)
    }

    fn inference(&self) -> InferenceModule {
        tracel_console::ProjectHandle::inference(self)
    }

    fn models(&self) -> Option<Models> {
        Some(tracel_console::ProjectHandle::models(self))
    }

    fn datasets(&self) -> Option<Datasets> {
        Some(tracel_console::ProjectHandle::datasets(self))
    }
}

#[cfg(feature = "station")]
impl Backend for tracel_station::Station {
    fn experiments(&self) -> ExperimentModule {
        tracel_station::Station::experiments(self)
    }

    fn models(&self) -> Option<Models> {
        Some(tracel_station::Station::models(self))
    }

    fn datasets(&self) -> Option<Datasets> {
        Some(tracel_station::Station::datasets(self))
    }
}

impl Backend for local::LocalBackend {
    fn experiments(&self) -> ExperimentModule {
        ExperimentModule::new(Arc::new(self.clone()))
    }
}
