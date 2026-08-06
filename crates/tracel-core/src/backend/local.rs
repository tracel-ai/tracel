//! Experiment backend that records locally to disk, no account required.

use std::{path::PathBuf, sync::Arc};

use crate::{
    context::{ContextError, IntoProviders, Providers},
    inference::DefaultInferenceProvider,
};

/// Experiment backend that records telemetry to a local directory instead of the Tracel cloud
/// or a Tracel Station.
///
/// Build one with [`LocalBackend::new`], then pass it to [`Context::new`](crate::Context::new).
/// It provides no model registry or dataset access, so
/// [`Context::models`](crate::Context::models) and [`Context::datasets`](crate::Context::datasets)
/// return `None`.
#[derive(Debug, Clone)]
pub struct LocalBackend {
    pub(crate) path: PathBuf,
}

impl IntoProviders for LocalBackend {
    fn into_providers(self) -> Result<Providers, ContextError> {
        let backend = Arc::new(self);
        Ok(Providers {
            experiment: backend,
            inference: Arc::new(DefaultInferenceProvider::new()),
            model_registry: None,
            dataset: None,
        })
    }
}

impl LocalBackend {
    /// Builds a `LocalBackend` that records under `path`, creating it if needed.
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}
