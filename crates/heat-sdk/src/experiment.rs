use crate::{error::HeatSDKError, websocket::WebSocketClient, ws_messages::WsMessage};

#[derive(Debug)]
pub struct Experiment {
    id: String,
    in_memory_logs: Vec<String>,
    ws_client: WebSocketClient,
}

impl Experiment {
    pub fn new(id: String, ws_client: WebSocketClient) -> Experiment {
        assert!(ws_client.state().is_open());
        Experiment {
            id,
            in_memory_logs: Vec::new(),
            ws_client: ws_client,
        }
    }

    pub fn add_log(&mut self, log: String) -> Result<(), HeatSDKError> {
        self.in_memory_logs.push(log.clone());
        self.ws_client.send(WsMessage::Log(log))?;

        Ok(())
    }

    pub fn id(&self) -> &String {
        &self.id
    }

    pub fn logs(&self) -> &Vec<String> {
        &self.in_memory_logs
    }
}

impl Drop for Experiment {
    fn drop(&mut self) {
        self.ws_client.close().unwrap();
    }
}
