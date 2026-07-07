use tracel_artifact::ReqwestTransferClient;
use tracel_client::StationClient;
use url::Url;

#[derive(Debug, Clone)]
pub struct StationBackend {
    pub client: StationClient,
    pub(crate) file_transfer_client: ReqwestTransferClient,
    pub(crate) model_cache: crate::model_registry::ModelCache,
}

impl StationBackend {
    pub fn create_context(url: Url) -> StationBackend {
        let host = url.host_str().unwrap_or("unknown");
        let station_id = match url.port() {
            Some(port) => format!("{host}_{port}"),
            None => host.to_string(),
        };

        let cache_root = directories::ProjectDirs::from("com", "tracel", "burncentral")
            .expect("could not determine cache directory")
            .cache_dir()
            .join("models")
            .join("station")
            .join(station_id);

        StationBackend {
            client: StationClient::from_url(url),
            file_transfer_client: ReqwestTransferClient::new(),
            model_cache: crate::model_registry::ModelCache::new(cache_root),
        }
    }
}
