use super::socket::ExperimentSocket;
use crate::api::EndExperimentStatus;
use crate::artifacts::ArtifactKind;
use crate::artifacts::{ArtifactDecode, ArtifactEncode, ArtifactScope, MemoryArtifactReader};
use crate::experiment::error::ExperimentTrackerError;
use crate::experiment::log_store::TempLogStore;
use crate::experiment::message::ExperimentMessage;
use crate::experiment::socket::ThreadError;
use crate::{api::Client, schemas::ExperimentPath, websocket::WebSocketClient};
use crossbeam::channel::Sender;
use std::ops::Deref;
use std::sync::{Arc, Weak};

/// Represents a handle to an experiment, allowing logging of artifacts, metrics, and messages.
#[derive(Clone, Debug)]
pub struct ExperimentRunHandle {
    recorder: Weak<ExperimentRunInner>,
}

impl ExperimentRunHandle {
    fn try_upgrade(&self) -> Result<Arc<ExperimentRunInner>, ExperimentTrackerError> {
        self.recorder
            .upgrade()
            .ok_or(ExperimentTrackerError::InactiveExperiment)
    }

    /// Log an artifact with the given name and kind.
    pub fn log_artifact<A: ArtifactEncode>(
        &self,
        name: impl Into<String>,
        kind: ArtifactKind,
        sources: A,
        settings: &A::Settings,
    ) -> Result<(), ExperimentTrackerError> {
        self.try_upgrade()?
            .log_artifact2(name, kind, sources, settings)
    }

    /// Loads an artifact with the given name and device.
    pub fn load_artifact<D: ArtifactDecode>(
        &self,
        name: impl AsRef<str>,
        settings: &D::Settings,
    ) -> Result<D, ExperimentTrackerError> {
        self.try_upgrade()?.load_artifact(name, settings)
    }

    /// Loads an artifact with the given name and device.
    pub fn load_artifact_reader(
        &self,
        name: impl AsRef<str>,
        experiment_num: u64,
    ) -> Result<MemoryArtifactReader, ExperimentTrackerError> {
        self.try_upgrade()?
            .load_artifact_reader(name, experiment_num)
    }

    /// Logs a metric with the given name, epoch, iteration, value, and group.
    pub fn log_metric(
        &self,
        name: impl Into<String>,
        epoch: usize,
        iteration: usize,
        value: f64,
        group: impl Into<String>,
    ) {
        self.try_log_metric(name, epoch, iteration, value, group)
            .expect("Failed to log metric, experiment may have been closed or inactive");
    }

    /// Attempts to log a metric with the given name, epoch, iteration, value, and group.
    pub fn try_log_metric(
        &self,
        name: impl Into<String>,
        epoch: usize,
        iteration: usize,
        value: f64,
        group: impl Into<String>,
    ) -> Result<(), ExperimentTrackerError> {
        self.try_upgrade()?
            .log_metric(name, epoch, iteration, value, group)
    }

    /// Logs an info message.
    pub fn log_info(&self, message: impl Into<String>) {
        self.try_log_info(message)
            .expect("Failed to log info, experiment may have been closed or inactive");
    }

    /// Attempts to log an info message.
    pub fn try_log_info(&self, message: impl Into<String>) -> Result<(), ExperimentTrackerError> {
        self.try_upgrade()?.log_info(message)
    }

    /// Logs an error message.
    pub fn log_error(&self, error: impl Into<String>) {
        self.try_log_error(error)
            .expect("Failed to log error, experiment may have been closed or inactive");
    }

    /// Attempts to log an error message.
    pub fn try_log_error(&self, error: impl Into<String>) -> Result<(), ExperimentTrackerError> {
        self.try_upgrade()?.log_error(error)
    }
}

/// Represents a recorder for an experiment, allowing logging of artifacts, metrics, and messages.
/// It is used internally by the [Experiment](ExperimentRun) struct to handle logging operations.
struct ExperimentRunInner {
    id: ExperimentPath,
    http_client: Client,
    sender: Sender<ExperimentMessage>,
}

impl ExperimentRunInner {
    fn send(&self, message: ExperimentMessage) -> Result<(), ExperimentTrackerError> {
        self.sender
            .send(message)
            .map_err(|_| ExperimentTrackerError::SocketClosed)
    }

    pub fn log_artifact2<A: ArtifactEncode>(
        &self,
        name: impl Into<String>,
        kind: ArtifactKind,
        artifact: A,
        settings: &A::Settings,
    ) -> Result<(), ExperimentTrackerError> {
        ArtifactScope::new(self.http_client.clone(), self.id.clone())
            .upload(name, kind, artifact, settings)
            .map_err(|e| {
                ExperimentTrackerError::InternalError(format!("Failed to log artifact: {e}"))
            })?;
        Ok(())
    }

    pub fn load_artifact_reader(
        &self,
        name: impl AsRef<str>,
        experiment_num: u64,
    ) -> Result<MemoryArtifactReader, ExperimentTrackerError> {
        let new_id = ExperimentPath::try_from(format!(
            "{}/{}/{}",
            self.id.owner_name(),
            self.id.project_name(),
            experiment_num
        ))
        .unwrap();
        ArtifactScope::new(self.http_client.clone(), new_id)
            .fetch(name.as_ref())
            .map_err(|e| {
                ExperimentTrackerError::InternalError(format!("Failed to load artifact: {e}"))
            })
    }

    pub fn load_artifact<D: ArtifactDecode>(
        &self,
        name: impl AsRef<str>,
        settings: &D::Settings,
    ) -> Result<D, ExperimentTrackerError> {
        let reader = ArtifactScope::new(self.http_client.clone(), self.id.clone())
            .fetch(name)
            .map_err(|e| {
                ExperimentTrackerError::InternalError(format!("Failed to load artifact: {e}"))
            })?;

        Ok(D::decode(&reader, settings).map_err(|e| {
            ExperimentTrackerError::InternalError(format!(
                "Failed to decode artifact: {}",
                e.into()
            ))
        })?)
    }

    pub fn log_metric(
        &self,
        name: impl Into<String>,
        epoch: usize,
        iteration: usize,
        value: f64,
        group: impl Into<String>,
    ) -> Result<(), ExperimentTrackerError> {
        let message = ExperimentMessage::MetricLog {
            name: name.into(),
            epoch,
            iteration,
            value,
            group: group.into(),
        };
        self.send(message)
    }

    pub fn log_info(&self, message: impl Into<String>) -> Result<(), ExperimentTrackerError> {
        self.send(ExperimentMessage::Log(message.into()))
    }

    pub fn log_error(&self, error: impl Into<String>) -> Result<(), ExperimentTrackerError> {
        self.send(ExperimentMessage::Error(error.into()))
    }
}

/// Represents an experiment in Burn Central, which is a run of a machine learning model or process.
pub struct ExperimentRun {
    inner: Arc<ExperimentRunInner>,
    socket: Option<ExperimentSocket>,
    // temporary field to allow dereferencing to handle
    _handle: ExperimentRunHandle,
}

impl ExperimentRun {
    pub fn new(
        http_client: Client,
        experiment_path: ExperimentPath,
    ) -> Result<Self, ExperimentTrackerError> {
        let mut ws_client = WebSocketClient::new();

        let ws_endpoint = http_client.format_websocket_url(
            experiment_path.owner_name(),
            experiment_path.project_name(),
            experiment_path.experiment_num(),
        );
        let cookie = http_client
            .get_session_cookie()
            .expect("Session cookie should be available");
        ws_client
            .connect(ws_endpoint, cookie)
            .map_err(|e| ExperimentTrackerError::ConnectionFailed(e.to_string()))?;

        let log_store = TempLogStore::new(http_client.clone(), experiment_path.clone());
        let (sender, receiver) = crossbeam::channel::unbounded();
        let socket = ExperimentSocket::new(ws_client, log_store, receiver);

        let inner = Arc::new(ExperimentRunInner {
            id: experiment_path.clone(),
            http_client: http_client.clone(),
            sender,
        });

        let _handle = ExperimentRunHandle {
            recorder: Arc::downgrade(&inner),
        };

        Ok(ExperimentRun {
            inner,
            socket: Some(socket),
            _handle,
        })
    }

    /// Returns a handle to the experiment, allowing logging of artifacts, metrics, and messages.
    pub fn handle(&self) -> ExperimentRunHandle {
        ExperimentRunHandle {
            recorder: Arc::downgrade(&self.inner),
        }
    }

    fn finish_internal(
        &mut self,
        end_status: EndExperimentStatus,
    ) -> Result<(), ExperimentTrackerError> {
        let thread_result = match self.socket.take() {
            Some(socket) => socket.close(),
            None => return Err(ExperimentTrackerError::AlreadyFinished),
        };

        match thread_result {
            Ok(_thread) => {}
            Err(ThreadError::WebSocket(msg)) => {
                eprintln!("Warning: WebSocket failure during experiment finish: {msg}");
            }
            Err(ThreadError::LogFlushError(msg)) => {
                eprintln!("Warning: Log flush failed: {msg}");
            }
            Err(ThreadError::MessageChannelClosed) => {
                eprintln!("Warning: Message channel closed before thread could complete");
            }
            Err(ThreadError::AbortError) => {
                return Err(ExperimentTrackerError::InternalError(
                    "Failed to abort thread.".into(),
                ));
            }
            Err(ThreadError::Panic) => {
                return Err(ExperimentTrackerError::InternalError(
                    "Experiment thread panicked".into(),
                ));
            }
        }

        self.inner
            .http_client
            .end_experiment(
                self.inner.id.owner_name(),
                self.inner.id.project_name(),
                self.inner.id.experiment_num(),
                end_status,
            )
            .map_err(|e| {
                ExperimentTrackerError::InternalError(format!("Failed to end experiment: {e}"))
            })?;

        Ok(())
    }

    pub fn finish(mut self) -> Result<(), ExperimentTrackerError> {
        self.finish_internal(EndExperimentStatus::Success)
    }

    pub fn fail(mut self, reason: impl Into<String>) -> Result<(), ExperimentTrackerError> {
        self.finish_internal(EndExperimentStatus::Fail(reason.into()))
    }
}

impl Drop for ExperimentRun {
    fn drop(&mut self) {
        let _ = self.finish_internal(EndExperimentStatus::Fail(
            "Experiment dropped without finishing".to_string(),
        ));
    }
}

/// Temporary implementation to allow dereferencing the Experiment to its recorder
/// This will be removed once the experiment logging api is completed
impl Deref for ExperimentRun {
    type Target = ExperimentRunHandle;

    fn deref(&self) -> &Self::Target {
        &self._handle
    }
}
