use burn_central_client::websocket::WebSocketError;
use burn_central_client::{ClientError, StationClient};
use url::Url;

use crate::context::{Backend, Context};

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub(crate) enum BurnStationError {
    Http(#[from] ClientError),
    WebSocket(#[from] WebSocketError),
}

#[derive(Debug, Clone)]
pub struct StationBackend {
    pub client: StationClient,
}

impl StationBackend {
    pub fn create_context(url: Url) -> Context {
        let backend = Backend::Station(StationBackend {
            client: StationClient::from_url(url),
        });
        Context::new(backend)
    }
}
