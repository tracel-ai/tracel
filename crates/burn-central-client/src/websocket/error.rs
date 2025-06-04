use thiserror::Error;

#[derive(Error, Debug)]
#[allow(clippy::enum_variant_names)]
pub enum WebSocketError {
    #[error("Connection Error: {0}")]
    ConnectionError(String),
    #[error("Send Error: {0}")]
    SendError(String),
    #[error("Unknown Error: {0}")]
    UnknownError(String),
}
