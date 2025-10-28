use std::{thread, time::Duration};

use reqwest::header::COOKIE;
use serde::Serialize;

use crate::websocket::WebSocketError;
use tungstenite::{
    Message, Utf8Bytes, WebSocket, client::IntoClientRequest, connect, stream::MaybeTlsStream,
};

const DEFAULT_RECONNECT_DELAY: Duration = Duration::from_millis(1000);

type Socket = WebSocket<MaybeTlsStream<std::net::TcpStream>>;
struct ConnectedSocket {
    socket: Socket,
    url: String,
    cookie: String,
}

pub struct WebSocketClient {
    state: Option<ConnectedSocket>,
}

impl WebSocketClient {
    pub fn new() -> Self {
        Self { state: None }
    }

    #[allow(dead_code)]
    pub fn is_connected(&self) -> bool {
        self.state.is_some()
    }

    pub fn connect(&mut self, url: String, session_cookie: &str) -> Result<(), WebSocketError> {
        let mut req = url
            .clone()
            .into_client_request()
            .expect("Should be able to create a client request from the URL");

        req.headers_mut().append(
            COOKIE,
            session_cookie
                .parse()
                .expect("Should be able to parse cookie header"),
        );

        let (socket, _) =
            connect(req).map_err(|e| WebSocketError::ConnectionError(e.to_string()))?;

        self.state = Some(ConnectedSocket {
            socket,
            url,
            cookie: session_cookie.to_string(),
        });
        Ok(())
    }

    pub fn reconnect(&mut self) -> Result<(), WebSocketError> {
        if let Some(socket) = self.state.take() {
            self.connect(socket.url, &socket.cookie)
        } else {
            Err(WebSocketError::CannotReconnect(
                "The websocket was never opened so it cannot be reconnected".to_string(),
            ))
        }
    }

    pub fn send<I: Serialize>(&mut self, message: I) -> Result<(), WebSocketError> {
        let socket = self.active_socket()?;

        let json = serde_json::to_string(&message)
            .map_err(|e| WebSocketError::SendError(e.to_string()))?;

        match Self::attempt_send(socket, &json) {
            Ok(_) => Ok(()),
            Err(_) => {
                thread::sleep(DEFAULT_RECONNECT_DELAY);
                self.reconnect()?;

                let socket = self.active_socket()?;
                Self::attempt_send(socket, &json)
            }
        }
    }

    fn attempt_send(socket: &mut Socket, payload: &str) -> Result<(), WebSocketError> {
        socket
            .send(Message::Text(Utf8Bytes::from(payload)))
            .map_err(|e| WebSocketError::SendError(e.to_string()))
    }

    pub fn close(&mut self) -> Result<(), WebSocketError> {
        let socket = self.active_socket()?;
        socket
            .close(None)
            .map_err(|e| WebSocketError::SendError(e.to_string()))
    }

    fn active_socket(&mut self) -> Result<&mut Socket, WebSocketError> {
        if let Some(socket) = self.state.as_mut() {
            Ok(&mut socket.socket)
        } else {
            Err(WebSocketError::NotConnected)
        }
    }
}

impl Drop for WebSocketClient {
    fn drop(&mut self) {
        _ = self.close();
    }
}
