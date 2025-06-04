use reqwest::header::COOKIE;
use serde::Serialize;

use tungstenite::{client::IntoClientRequest, connect, stream::MaybeTlsStream, Message, WebSocket};

use super::WebSocketError;

type Socket = WebSocket<MaybeTlsStream<std::net::TcpStream>>;

pub enum SocketStatus {
    Open,
    Closed,
}

impl SocketStatus {
    pub fn is_open(&self) -> bool {
        match self {
            SocketStatus::Open => true,
            SocketStatus::Closed => false,
        }
    }
}

#[derive(Debug)]
pub struct WebSocketClient {
    state: Option<Socket>,
}

impl WebSocketClient {
    pub fn new() -> WebSocketClient {
        WebSocketClient { state: None }
    }

    pub fn connect(&mut self, url: String, cookie: &str) -> Result<(), WebSocketError> {
        let mut req = url.into_client_request().unwrap();
        let headers = req.headers_mut();
        headers.append(COOKIE, cookie.parse().unwrap());
        let (socket, _) = connect(req)
            // .await
            .map_err(|e| WebSocketError::ConnectionError(e.to_string()))?;

        self.state = Some(socket);

        Ok(())
    }

    pub fn send<I: Serialize>(&mut self, message: I) -> Result<(), WebSocketError> {
        if let Some(socket) = &mut self.state {
            socket
                .send(Message::Text(serde_json::to_string(&message).unwrap().into()))
                .map_err(|e| WebSocketError::SendError(e.to_string()))?;
        }

        Ok(())
    }

    pub fn close(&mut self) -> Result<(), WebSocketError> {
        if let Some(socket) = &mut self.state {
            socket
                .close(None)
                .map_err(|e| WebSocketError::SendError(e.to_string()))?;
        }

        Ok(())
    }

    pub fn state(&self) -> SocketStatus {
        if self.state.is_some() {
            SocketStatus::Open
        } else {
            SocketStatus::Closed
        }
    }
}
