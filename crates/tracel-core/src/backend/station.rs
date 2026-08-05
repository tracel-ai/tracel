use std::sync::Arc;

use tracel_artifact::ReqwestTransferClient;
use tracel_client::StationClient;
use url::Url;

use crate::{
    context::{ContextError, IntoProviders, Providers},
    inference::DefaultInferenceProvider,
};

#[derive(Debug, thiserror::Error)]
pub enum StationError {
    #[error("could not determine a cache directory for downloaded models")]
    NoCacheDir,
}

#[derive(Clone)]
pub struct StationBackend {
    pub client: StationClient,
    pub file_transfer_client: ReqwestTransferClient,
    pub model_cache: crate::model_registry::ModelCache,
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
