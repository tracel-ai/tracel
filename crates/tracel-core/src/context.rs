//! The [`Context`] is the SDK's single entry point: build one from a backend
//! ([`CloudBackend`](crate::CloudBackend), [`LocalBackend`](crate::LocalBackend), or
//! [`StationBackend`](crate::StationBackend)), then use it to reach every other module.
//!
//! ```rust,no_run
//! use tracel_core::{Context, LocalBackend};
//!
//! # fn example() -> anyhow::Result<()> {
//! let ctx = Context::new(LocalBackend::new("./runs".into()))?;
//! let experiments = ctx.experiment();
//! # Ok(())
//! # }
//! ```
//!
//! A backend does not need to support every module: [`Context::models`] and [`Context::datasets`]
//! return `None` for backends that don't provide a model registry or dataset access
//! (see each backend's docs for what it supports).

use std::sync::Arc;

use crate::backend::cloud::CloudError;
#[cfg(feature = "station")]
use crate::backend::station::StationError;
use crate::dataset::{DatasetModule, DatasetProvider};
use crate::model_registry::{ModelRegistryModule, ModelRegistryProvider};
use tracel_experiment::ExperimentModule;
use tracel_experiment::ExperimentProvider;
use tracel_inference::{InferenceModule, InferenceProvider};

/// Errors that can occur while constructing a [`Context`] from a backend.
#[derive(Debug, thiserror::Error)]
pub enum ContextError {
    /// Construction of a [`CloudBackend`](crate::CloudBackend) failed.
    #[error(transparent)]
    Cloud(#[from] CloudError),
    /// Construction of a [`StationBackend`](crate::StationBackend) failed.
    #[cfg(feature = "station")]
    #[error(transparent)]
    Station(#[from] StationError),
}

/// The SDK's entry point, built from a backend with [`Context::new`].
///
/// A `Context` gives access to each module the current backend supports: [`Context::experiment`]
/// and [`Context::inference`] are always available, while [`Context::models`] and
/// [`Context::datasets`] return `None` for backends without a model registry or dataset access.
#[derive(Clone)]
pub struct Context {
    experiment_provider: Arc<dyn ExperimentProvider>,
    inference_provider: Arc<dyn InferenceProvider>,
    model_registry_provider: Option<Arc<dyn ModelRegistryProvider>>,
    dataset_provider: Option<Arc<dyn DatasetProvider>>,
}

pub(crate) struct Providers {
    pub(crate) experiment: Arc<dyn ExperimentProvider>,
    pub(crate) inference: Arc<dyn InferenceProvider>,
    pub(crate) model_registry: Option<Arc<dyn ModelRegistryProvider>>,
    pub(crate) dataset: Option<Arc<dyn DatasetProvider>>,
}

pub(crate) trait IntoProviders {
    fn into_providers(self) -> Result<Providers, ContextError>;
}

impl Context {
    /// Builds a `Context` from a backend, e.g. [`CloudBackend`](crate::CloudBackend),
    /// [`LocalBackend`](crate::LocalBackend), or [`StationBackend`](crate::StationBackend).
    #[allow(private_bounds)]
    pub fn new(connection: impl IntoProviders) -> Result<Self, ContextError> {
        let providers = connection.into_providers()?;
        Ok(Self {
            experiment_provider: providers.experiment,
            inference_provider: providers.inference,
            model_registry_provider: providers.model_registry,
            dataset_provider: providers.dataset,
        })
    }

    /// Entry point for creating and running experiments.
    pub fn experiment(&self) -> ExperimentModule {
        ExperimentModule::new(self.experiment_provider.clone())
    }

    /// Entry point for running inference against a deployed model.
    pub fn inference(&self) -> InferenceModule {
        InferenceModule::new(self.inference_provider.clone())
    }

    /// Entry point for uploading and downloading models, or `None` if the backend has no model
    /// registry.
    pub fn models(&self) -> Option<ModelRegistryModule> {
        self.model_registry_provider
            .clone()
            .map(ModelRegistryModule::new)
    }

    /// Entry point for reading datasets, or `None` if the backend has no dataset access.
    pub fn datasets(&self) -> Option<DatasetModule> {
        self.dataset_provider.clone().map(DatasetModule::new)
    }
}
