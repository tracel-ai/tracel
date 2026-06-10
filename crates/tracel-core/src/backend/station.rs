use burn_central_client::websocket::WebSocketError;
use burn_central_client::{ClientError, StationClient};
use url::Url;

#[derive(Debug, thiserror::Error)]
pub(crate) enum StationError {
    #[error("Failed to create experiment on Station — check your Station URL and connectivity")]
    ExperimentCreation(#[from] ClientError),
    #[error("Failed to establish WebSocket connection to Station")]
    WebSocket(#[from] WebSocketError),
}

#[derive(Debug, Clone)]
pub struct StationBackend {
    pub client: StationClient,
}

impl StationBackend {
    pub fn create_context(url: Url) -> StationBackend {
        StationBackend {
            client: StationClient::from_url(url),
        }
    }
}
