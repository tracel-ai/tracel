
use crate::{error::HeatSdkError, websocket::WebSocketClient};
use std::sync::mpsc;

use super::{thread::ExperimentWSHandler, WsMessage};

#[derive(Debug)]
pub struct Experiment {
    id: String,
    in_memory_logs: Option<Vec<String>>,
    // ws_client: WebSocketClient,
    handler: Option<ExperimentWSHandler>,
    // scheduler_checkpoint_handler: Option<ExperimentCheckpointerHandler>,
    // optimizer_checkpoint_handler: Option<ExperimentCheckpointerHandler>,
    // experiment_checkpoint_handler: Option<ExperimentCheckpointerHandler>,
}

impl Experiment {
    pub fn new(id: String, ws_client: WebSocketClient) -> Experiment {
        assert!(ws_client.state().is_open());

        let handler = ExperimentWSHandler::new(ws_client);

        Experiment {
            id,
            in_memory_logs: Some(Vec::new()),
            // ws_client: ws_client,
            handler: Some(handler),
        }
    }

    pub fn add_log(&mut self, log: String) -> Result<(), HeatSdkError> {
        // self.in_memory_logs.push(log.clone());
        // self.ws_client.send(WsMessage::Log(log))?;

        Ok(())
    }

    pub fn id(&self) -> &String {
        &self.id
    }

    pub fn try_logs(&self) -> Result<Vec<String>, HeatSdkError> {
        if let Some(logs) = &self.in_memory_logs {
            Ok(logs.clone())
        } else {
            Err(HeatSdkError::ClientError("Logs not yet available".to_string()))
        }
    }

    pub fn get_ws_sender(&self) -> Result<mpsc::Sender<WsMessage>, HeatSdkError> {
        if let Some(handler) = &self.handler {
            Ok(handler.get_sender())
        } else {
            Err(HeatSdkError::ClientError("Experiment handling thread not started".to_string()))
        }
    }

    pub fn stop(&mut self) {
        // self.ws_client.close().unwrap();
        
        if let Some(handler) = self.handler.take() {
            let result = handler.join();
            self.in_memory_logs.replace(result.logs);
        }
    }
    

}

impl Drop for Experiment {
    fn drop(&mut self) {
        // self.ws_client.close().unwrap();
        self.stop();
    }
}
