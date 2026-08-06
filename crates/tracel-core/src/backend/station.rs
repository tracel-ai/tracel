//! Experiment backend running against a Tracel Station.

use std::sync::Arc;

use tracel_artifact::ReqwestTransferClient;
use tracel_client::StationClient;
use url::Url;

use crate::{
    context::{ContextError, IntoProviders, Providers},
    inference::DefaultInferenceProvider,
    model_registry::ModelCache,
};

/// Errors from [`StationBackend`] construction.
#[derive(Debug, thiserror::Error)]
pub enum StationError {
    /// No local cache directory could be resolved for downloaded models.
    #[error("could not determine a cache directory for downloaded models")]
    NoCacheDir,
}

/// Experiment backend that ships telemetry to a Tracel Station.
///
/// Build one with [`StationBackend::new`], then pass it to [`Context::new`](crate::Context::new).
/// Unlike [`CloudBackend`](crate::CloudBackend), it provides dataset access in addition to
/// experiments and a model registry.
#[derive(Clone)]
pub struct StationBackend {
    pub(crate) client: StationClient,
    pub(crate) file_transfer_client: ReqwestTransferClient,
    pub(crate) model_cache: ModelCache,
}

impl IntoProviders for StationBackend {
    fn into_providers(self) -> Result<Providers, ContextError> {
        let backend = Arc::new(self);
        Ok(Providers {
            experiment: backend.clone(),
            inference: Arc::new(DefaultInferenceProvider::new()),
            model_registry: Some(backend.clone()),
            dataset: Some(backend),
        })
    }
}

impl StationBackend {
    /// Builds a `StationBackend` for the Tracel Station at `url`.
    pub fn new(url: Url) -> Result<Self, StationError> {
        let host = url.host_str().unwrap_or("unknown");
        let station_id = match url.port() {
            Some(port) => format!("{host}_{port}"),
            None => host.to_string(),
        };

        let cache_root = crate::resolve_cache_dir()
            .ok_or(StationError::NoCacheDir)?
            .join("station")
            .join(station_id)
            .join("models");

        Ok(StationBackend {
            client: StationClient::from_url(url),
            file_transfer_client: ReqwestTransferClient::new(),
            model_cache: crate::model_registry::ModelCache::new(cache_root),
        })
    }
}
