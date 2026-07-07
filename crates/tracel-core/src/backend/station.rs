use tracel_artifact::ReqwestTransferClient;
use tracel_client::StationClient;
use url::Url;

#[derive(Debug, Clone)]
pub struct StationBackend {
    pub client: StationClient,
    pub(crate) file_transfer_client: ReqwestTransferClient,
}

impl StationBackend {
    pub fn create_context(url: Url) -> StationBackend {
        StationBackend {
            client: StationClient::from_url(url),
            file_transfer_client: ReqwestTransferClient::new(),
        }
    }
}
