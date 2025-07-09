use reqwest::header::COOKIE;
use serde::Serialize;

use crate::websocket::WebSocketError;
use tungstenite::{
    Message, Utf8Bytes, WebSocket, client::IntoClientRequest, connect, stream::MaybeTlsStream,
};

type Socket = WebSocket<MaybeTlsStream<std::net::TcpStream>>;

pub struct WebSocketClient {
    state: Option<Socket>,
}

impl WebSocketClient {
    pub fn new() -> Self {
        Self { state: None }
    }

    pub fn is_connected(&self) -> bool {
        self.state.is_some()
    }

    pub fn connect(&mut self, url: String, cookie: &str) -> Result<(), WebSocketError> {
        let mut req = url
            .into_client_request()
            .expect("Should be able to create a client request from the URL");

        req.headers_mut().append(
            COOKIE,
            cookie
                .parse()
                .expect("Should be able to parse cookie header"),
        );

        let (socket, _) =
            connect(req).map_err(|e| WebSocketError::ConnectionError(e.to_string()))?;

        self.state = Some(socket);
        Ok(())
    }

    pub fn send<I: Serialize>(&mut self, message: I) -> Result<(), WebSocketError> {
        let socket = self.state.as_mut().ok_or(WebSocketError::NotConnected)?;

        let json = serde_json::to_string(&message)
            .map_err(|e| WebSocketError::SendError(e.to_string()))?;

        socket
            .send(Message::Text(Utf8Bytes::from(json)))
            .map_err(|e| WebSocketError::SendError(e.to_string()))
    }

    pub fn close(&mut self) -> Result<(), WebSocketError> {
        let socket = self.state.as_mut().ok_or(WebSocketError::NotConnected)?;
        socket
            .close(None)
            .map_err(|e| WebSocketError::SendError(e.to_string()))
    }
}

impl Drop for WebSocketClient {
    fn drop(&mut self) {
        _ = self.close();
    }
}
