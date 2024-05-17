use serde::Serialize;

use tungstenite::{stream::MaybeTlsStream, *};

use super::WebSocketError;

type Socket = WebSocket<MaybeTlsStream<std::net::TcpStream>>;

#[derive(Debug)]
pub struct WebSocketClient {
    state: Option<Socket>,
}

impl WebSocketClient {
    pub fn new() -> WebSocketClient {
        WebSocketClient { state: None }
    }

    pub fn connect(&mut self, url: String) -> Result<(), WebSocketError> {
        let (socket, _) = connect(url)
            // .await
            .map_err(|e| WebSocketError::ConnectionError(e.to_string()))?;

        self.state = Some(socket);

        Ok(())
    }

    pub fn send<I: Serialize>(&mut self, message: I) -> Result<(), WebSocketError> {
        if let Some(socket) = &mut self.state {
            socket
                .send(Message::Text(serde_json::to_string(&message).unwrap()))
                .map_err(|e| WebSocketError::SendError(e.to_string()))?;
        }

        Ok(())
    }
}