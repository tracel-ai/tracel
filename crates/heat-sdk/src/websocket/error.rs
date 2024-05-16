use thiserror::Error;

#[derive(Error, Debug)]
pub enum WebSocketError {
    #[error("Connection Error: {0}")]
    ConnectionError(String),
    #[error("Send Error: {0}")]
    SendError(String),
    #[error("Unknown Error: {0}")]
    UnknownError(String),
}
