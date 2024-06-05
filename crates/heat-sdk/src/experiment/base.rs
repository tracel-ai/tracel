use reqwest::header::COOKIE;

use crate::{error::HeatSdkError, http_schemas::URLSchema, websocket::WebSocketClient};
use std::sync::mpsc;

use super::{thread::ExperimentWSHandler, WsMessage};

#[derive(Debug)]
pub struct TempLogStore {
    logs: Vec<String>,
    http_client: reqwest::blocking::Client,
    endpoint: String,
    exp_id: String,
    bytes: usize,
    session_cookie: String,
}

impl TempLogStore {
    // 100 MiB
    const BYTE_LIMIT: usize = 100 * 1024 * 1024;

    pub fn new(
        http_client: reqwest::blocking::Client,
        endpoint: String,
        exp_id: String,
        session_cookie: String,
    ) -> TempLogStore {
        TempLogStore {
            logs: Vec::new(),
            http_client,
            endpoint,
            exp_id,
            bytes: 0,
            session_cookie,
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
            let logs_upload_url = self
                .http_client
                .post(format!(
                    "{}/experiments/{}/logs",
                    self.endpoint, self.exp_id
                ))
                .header(COOKIE, self.session_cookie.clone())
                .send()?
                .json::<URLSchema>()?
                .url;

            self.http_client
                .put(logs_upload_url)
                .body(self.logs.join(""))
                .send()?;

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
