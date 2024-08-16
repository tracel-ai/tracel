use crate::{errors::sdk::HeatSdkError, http::HttpClient, websocket::WebSocketClient};
use std::sync::mpsc;

use super::{thread::ExperimentWSHandler, WsMessage};

#[derive(Debug)]
pub struct TempLogStore {
    logs: Vec<String>,
    http_client: HttpClient,
    exp_id: String,
    bytes: usize,
}

impl TempLogStore {
    // 100 MiB
    const BYTE_LIMIT: usize = 100 * 1024 * 1024;

    pub fn new(http_client: HttpClient, exp_id: String) -> TempLogStore {
        TempLogStore {
            logs: Vec::new(),
            http_client,
            exp_id,
            bytes: 0,
        }
    }

    pub fn push(&mut self, log: String) -> Result<(), HeatSdkError> {
        if self.bytes + log.len() > Self::BYTE_LIMIT {
            self.flush()?;
        }

        self.bytes += log.len();
        self.logs.push(log);

        Ok(())
    }

    pub fn flush(&mut self) -> Result<(), HeatSdkError> {
        if !self.logs.is_empty() {
            let logs_upload_url = self.http_client.request_logs_upload_url(&self.exp_id)?;
            self.http_client
                .upload_bytes_to_url(&logs_upload_url, self.logs.join("").into_bytes())?;

            self.logs.clear();
            self.bytes = 0;
        }

        Ok(())
    }
}

#[derive(Debug)]
pub struct Experiment {
    id: String,
    handler: Option<ExperimentWSHandler>,
}

impl Experiment {
    pub fn new(id: String, ws_client: WebSocketClient, log_store: TempLogStore) -> Experiment {
        assert!(ws_client.state().is_open());

        let handler = ExperimentWSHandler::new(ws_client, log_store);

        Experiment {
            id,
            handler: Some(handler),
        }
    }

    pub fn id(&self) -> &String {
        &self.id
    }

    pub fn get_ws_sender(&self) -> Result<mpsc::Sender<WsMessage>, HeatSdkError> {
        if let Some(handler) = &self.handler {
            Ok(handler.get_sender())
        } else {
            Err(HeatSdkError::ClientError(
                "Experiment handling thread not started".to_string(),
            ))
        }
    }

    pub fn stop(&mut self) {
        if let Some(handler) = self.handler.take() {
            let result = handler.join();
            let mut logs = result.logs;
            logs.flush().expect("Should be able to flush logs");
        }
    }
}

impl Drop for Experiment {
    fn drop(&mut self) {
        self.stop();
    }
}
