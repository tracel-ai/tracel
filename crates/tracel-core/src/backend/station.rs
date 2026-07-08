use tracel_artifact::ReqwestTransferClient;
use tracel_client::StationClient;
use url::Url;

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

impl StationBackend {
    pub fn create_context(url: Url) -> Result<StationBackend, StationError> {
        let host = url.host_str().unwrap_or("unknown");
        let station_id = match url.port() {
            Some(port) => format!("{host}_{port}"),
            None => host.to_string(),
        };

        let cache_root = crate::model_registry::resolve_cache_dir()
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
