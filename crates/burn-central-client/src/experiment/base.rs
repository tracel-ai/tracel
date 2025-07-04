use super::{ExperimentMessage, socket::ExperimentSocket};
use crate::http::EndExperimentStatus;
use crate::{error::BurnCentralClientError, http::HttpClient, schemas::ExperimentPath, websocket::WebSocketClient, ArtifactKind, ArtifactLoadArgs, ArtifactRecordArgs, ArtifactRecorder};
use std::ops::Deref;
use std::sync::mpsc;
use burn::prelude::Backend;
use burn::record::{FullPrecisionSettings, Record, Recorder, RecorderError};
use serde::de::DeserializeOwned;
use serde::Serialize;
use strum::EnumString;

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

#[derive(Clone, Debug)]
pub struct ExperimentHandle {
    id: ExperimentPath,
    http_client: HttpClient,
    sender: mpsc::Sender<ExperimentMessage>,
}

impl ExperimentHandle {
    pub fn id(&self) -> &ExperimentPath {
        &self.id
    }

    pub fn log_artifact<B: Backend>(
        &self,
        name: impl Into<String>,
        kind: ArtifactKind,
        record: impl Record<B>,
    ) {
        let recorder = ArtifactRecorder::new(self.http_client.clone());
        let args = ArtifactRecordArgs {
            experiment_path: self.id.clone(),
            name: name.into(),
            kind,
        };
        let res = recorder.record(record, args);
        if let Err(e) = res {
            eprintln!("Failed to log artifact: {}", e);
        }
    }

    pub fn load_artifact<B: Backend, R: Record<B>>(
        &self,
        name: impl Into<String>,
        device: B::Device
    ) -> Result<R, RecorderError> {
        let recorder = ArtifactRecorder::new(self.http_client.clone());
        let args = ArtifactLoadArgs {
            experiment_path: self.id.clone(),
            name: name.into(),
        };
        recorder.load(args, &device)
    }

    pub fn log_metric(
        &self,
        name: impl Into<String>,
        epoch: usize,
        iteration: usize,
        value: f64,
        group: impl Into<String>,
    ) {
        let message = ExperimentMessage::MetricLog {
            name: name.into(),
            epoch,
            iteration,
            value,
            group: group.into(),
        };
        _ = self.sender.send(message);
    }

    pub fn log_info(&self, message: impl Into<String>) {
        let message = ExperimentMessage::Log(message.into());
        _ = self.sender.send(message);
    }

    pub fn log_error(&self, error: impl Into<String>) {
        let message = ExperimentMessage::Error(error.into());
        _ = self.sender.send(message);
    }
}

#[derive(Debug)]
pub struct Experiment {
    experiment_path: ExperimentPath,
    http_client: HttpClient,
    socket: Option<ExperimentSocket>,
    handle: ExperimentHandle,
}

impl Experiment {
    pub fn new(
        http_client: HttpClient,
        experiment_path: ExperimentPath,
    ) -> Result<Self, BurnCentralClientError> {
        let mut ws_client = WebSocketClient::new();

        let ws_endpoint = http_client.format_websocket_url(
            &experiment_path.owner_name(),
            &experiment_path.project_name(),
            experiment_path.experiment_num(),
        );
        let cookie = http_client
            .get_session_cookie()
            .expect("Session cookie should be available");
        ws_client.connect(ws_endpoint, cookie)?;

        let log_store = TempLogStore::new(http_client.clone(), experiment_path.clone());

        let socket = ExperimentSocket::new(ws_client, log_store);

        let handle = ExperimentHandle {
            id: experiment_path.clone(),
            http_client: http_client.clone(),
            sender: socket.sender().clone(),
        };

        Ok(Experiment {
            experiment_path,
            http_client,
            socket: Some(socket),
            handle,
        })
    }

    pub fn handle(&self) -> ExperimentHandle {
        self.handle.clone()
    }

    pub(crate) fn finish_internal(
        &mut self,
        end_status: EndExperimentStatus,
    ) -> Result<(), BurnCentralClientError> {
        if let Some(socket) = self.socket.take() {
            let mut res = socket.close().map_err(|e| {
                BurnCentralClientError::UnknownError(
                    "Failed to close the experiment socket".to_string(),
                )
            })?;
            res.logs.flush()?;
        }

        // End the experiment in the backend
        self.http_client.end_experiment(
            self.experiment_path.owner_name(),
            self.experiment_path.project_name(),
            self.experiment_path.experiment_num(),
            end_status,
        )?;

        Ok(())
    }

    pub fn finish(mut self, status: EndExperimentStatus) -> Result<(), BurnCentralClientError> {
        self.finish_internal(status)
    }
}

impl Drop for Experiment {
    fn drop(&mut self) {
        let _ = self.finish_internal(EndExperimentStatus::Fail(
            "Experiment dropped without finishing".to_string(),
        ));
    }
}

impl Deref for Experiment {
    type Target = ExperimentHandle;

    fn deref(&self) -> &Self::Target {
        &self.handle
    }
}

#[test]
fn test_experiment_handle() {
    let experiment = Experiment::new(
        HttpClient::new_without_credentials("http://localhost:8000".parse().unwrap()),
        ExperimentPath::try_from("test_owner/test_project/1".to_string()).unwrap(),
    )
        .unwrap();

    let handle = experiment.log_error("a");
    experiment.log_info("ad");
    let handle = experiment.handle();

    handle.log_metric("accuracy", 1, 1, 0.95, "test");
    handle.log_info("Training started");
    handle.log_error("An error occurred");
    handle.log_metric("accuracy", 1, 1, 0.95, "test");

    let a = experiment.finish(EndExperimentStatus::Success);
}
