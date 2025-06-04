use crate::{
    errors::client::BurnCentralClientError, http::HttpClient, schemas::ExperimentPath,
    websocket::WebSocketClient,
};
use std::sync::mpsc;

use super::{WsMessage, thread::ExperimentWSHandler};

#[derive(Debug)]
pub struct TempLogStore {
    logs: Vec<String>,
    http_client: HttpClient,
    experiment_path: ExperimentPath,
    bytes: usize,
}

impl TempLogStore {
    // 100 MiB
    const BYTE_LIMIT: usize = 100 * 1024 * 1024;

    pub fn new(http_client: HttpClient, experiment_path: ExperimentPath) -> TempLogStore {
        TempLogStore {
            logs: Vec::new(),
            http_client,
            experiment_path,
            bytes: 0,
        }
    }

    pub fn push(&mut self, log: String) -> Result<(), BurnCentralClientError> {
        if self.bytes + log.len() > Self::BYTE_LIMIT {
            self.flush()?;
        }

        self.bytes += log.len();
        self.logs.push(log);

        Ok(())
    }

    pub fn flush(&mut self) -> Result<(), BurnCentralClientError> {
        if !self.logs.is_empty() {
            let logs_upload_url = self.http_client.request_logs_upload_url(
                self.experiment_path.owner_name(),
                self.experiment_path.project_name(),
                self.experiment_path.experiment_num(),
            )?;
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
    experiment_path: ExperimentPath,
    handler: Option<ExperimentWSHandler>,
}

impl Experiment {
    pub fn new(
        experiment_path: ExperimentPath,
        ws_client: WebSocketClient,
        log_store: TempLogStore,
    ) -> Experiment {
        assert!(ws_client.state().is_open());

        let handler = ExperimentWSHandler::new(ws_client, log_store);

        Experiment {
            experiment_path,
            handler: Some(handler),
        }
    }

    pub fn experiment_path(&self) -> &ExperimentPath {
        &self.experiment_path
    }

    pub(crate) fn get_ws_sender(&self) -> Result<mpsc::Sender<WsMessage>, BurnCentralClientError> {
        if let Some(handler) = &self.handler {
            Ok(handler.get_sender())
        } else {
            Err(BurnCentralClientError::UnknownError(
                "Experiment not started yet".to_string(),
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
