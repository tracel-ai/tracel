use burn_central_client::websocket::WebSocketError;
use burn_central_client::{ClientError, StationClient};
use url::Url;

use crate::context::{Backend, Context};

#[derive(Debug, thiserror::Error)]
pub(crate) enum StationError {
    #[error("Failed to create experiment on Station — check your Station URL and connectivity")]
    ExperimentCreation(#[from] ClientError),
    #[error("Failed to establish WebSocket connection to Station")]
    WebSocket(#[from] WebSocketError),
}

#[derive(Debug, Clone)]
pub struct StationBackend {
    pub(crate) client: StationClient,
}

impl StationBackend {
    pub(crate) fn create_context(url: Url) -> Context {
        let backend = Backend::Station(StationBackend {
            client: StationClient::from_url(url),
        });
        Context::new(backend)
    }
}
