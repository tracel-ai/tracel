use thiserror::Error;

#[derive(Error, Debug)]
#[allow(clippy::enum_variant_names)]
pub enum WebSocketError {
    #[error("Failed to connect WebSocket: {0}")]
    ConnectionError(String),
    #[error("WebSocket send error: {0}")]
    SendError(String),
    #[error("WebSocket is not connected")]
    NotConnected,
    #[error("WebSocket cannot reconnect: {0}")]
    CannotReconnect(String),
}
