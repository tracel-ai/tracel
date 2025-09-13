use super::socket::ExperimentSocket;
use crate::api::EndExperimentStatus;
use crate::experiment::error::ExperimentTrackerError;
use crate::experiment::log_store::TempLogStore;
use crate::experiment::message::ExperimentMessage;
use crate::experiment::socket::ThreadError;
use crate::record::{
    ArtifactKind, ArtifactLoadArgs, ArtifactQueryArgs, ArtifactRecordArgs, ArtifactRecorder,
};
use crate::{api::Client, schemas::ExperimentPath, websocket::WebSocketClient};
use burn::prelude::Backend;
use burn::record::{Record, Recorder};
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

    /// Logs an artifact with the given name and kind.
    pub fn log_artifact<B: Backend>(
        &self,
        name: impl Into<String>,
        kind: ArtifactKind,
        record: impl Record<B>,
    ) {
        self.try_log_artifact(name, kind, record)
            .expect("Failed to log artifact, experiment may have been closed or inactive");
    }

    /// Attempts to log an artifact with the given name and kind.
    pub fn try_log_artifact<B: Backend>(
        &self,
        name: impl Into<String>,
        kind: ArtifactKind,
        record: impl Record<B>,
    ) -> Result<(), ExperimentTrackerError> {
        self.try_upgrade()?.log_artifact(name, kind, record)
    }

    /// Loads an artifact with the given name and device.
    pub fn load_artifact<B, R>(
        &self,
        name: impl Into<String>,
        device: &B::Device,
    ) -> Result<R, ExperimentTrackerError>
    where
        B: Backend,
        R: Record<B>,
    {
        self.try_upgrade()?.load_artifact(name, device)
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

    pub fn log_artifact<B: Backend>(
        &self,
        name: impl Into<String>,
        kind: ArtifactKind,
        record: impl Record<B>,
    ) -> Result<(), ExperimentTrackerError> {
        let recorder = ArtifactRecorder::new(self.http_client.clone());
        let args = ArtifactRecordArgs {
            experiment_path: self.id.clone(),
            name: name.into(),
            kind,
        };
        recorder
            .record(record, args)
            .map_err(ExperimentTrackerError::BurnRecorderError)
    }

    pub fn load_artifact<B: Backend, R: Record<B>>(
        &self,
        name: impl Into<String>,
        device: &B::Device,
    ) -> Result<R, ExperimentTrackerError> {
        let recorder = ArtifactRecorder::new(self.http_client.clone());
        let args = ArtifactLoadArgs {
            experiment_path: self.id.clone(),
            query: ArtifactQueryArgs::ByName(name.into()),
        };
        recorder
            .load(args, device)
            .map_err(ExperimentTrackerError::BurnRecorderError)
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

#[cfg(test)]
#[allow(dead_code)]
mod test {
    use crate::api::Client;
    use crate::experiment::ExperimentRun;
    use crate::record::ArtifactKind;
    use crate::schemas::ExperimentPath;
    use burn::backend::NdArray;
    use burn::nn::conv::{Conv2d, Conv2dConfig};
    use burn::nn::pool::{AdaptiveAvgPool2d, AdaptiveAvgPool2dConfig};
    use burn::nn::{Dropout, DropoutConfig, Linear, LinearConfig, Relu};
    use burn::prelude::*;

    #[derive(Module, Debug)]
    pub struct Model<B: Backend> {
        conv1: Conv2d<B>,
        conv2: Conv2d<B>,
        pool: AdaptiveAvgPool2d,
        dropout: Dropout,
        linear1: Linear<B>,
        linear2: Linear<B>,
        activation: Relu,
    }

    #[derive(Config, Debug)]
    pub struct ModelConfig {
        num_classes: usize,
        hidden_size: usize,
        #[config(default = "0.5")]
        dropout: f64,
    }

    impl ModelConfig {
        pub fn init<B: Backend>(&self, device: &B::Device) -> Model<B> {
            Model {
                conv1: Conv2dConfig::new([1, 8], [3, 3]).init(device),
                conv2: Conv2dConfig::new([8, 16], [3, 3]).init(device),
                pool: AdaptiveAvgPool2dConfig::new([8, 8]).init(),
                activation: Relu::new(),
                linear1: LinearConfig::new(16 * 8 * 8, self.hidden_size).init(device),
                linear2: LinearConfig::new(self.hidden_size, self.num_classes).init(device),
                dropout: DropoutConfig::new(self.dropout).init(),
            }
        }
    }

    fn test_experiment_handle() -> anyhow::Result<()> {
        type TestBackend = NdArray;
        type TestDevice = <TestBackend as Backend>::Device;

        let device = TestDevice::default();
        train_experiment::<TestBackend>(device)?;

        Ok(())
    }

    fn train_experiment<B: Backend>(device: B::Device) -> anyhow::Result<()> {
        let experiment = ExperimentRun::new(
            Client::new_without_credentials("http://localhost:9001".parse().unwrap()),
            ExperimentPath::try_from("test_owner/test_project/1".to_string()).unwrap(),
        )?;

        let model_config = ModelConfig::new(10, 128);
        let model = model_config.init::<B>(&device);

        experiment.try_log_error("This is an error")?;
        experiment.try_log_info("Hello, world")?;
        experiment.try_log_artifact("model", ArtifactKind::Model, model.clone().into_record())?;

        std::thread::spawn({
            let handle = experiment.handle();
            let model_fork = model.clone();
            move || {
                handle.log_metric("accuracy", 1, 1, 0.95, "test");
                handle.log_info("Training started");
                handle.log_error("An error occurred");
                handle.log_metric("accuracy", 1, 1, 0.95, "test");
                handle.log_artifact("model_fork", ArtifactKind::Model, model_fork.into_record());
            }
        });

        experiment.finish()?;

        Ok(())
    }

    fn eval_experiment<B: Backend>(device: B::Device) -> anyhow::Result<()> {
        let experiment = ExperimentRun::new(
            Client::new_without_credentials("http://localhost:8000".parse().unwrap()),
            ExperimentPath::try_from("test_owner/test_project/1".to_string()).unwrap(),
        )?;

        let model_config = ModelConfig::new(10, 128);

        // load the artifact back
        let loaded_model_record = experiment.load_artifact("model", &device)?;

        let _model = model_config
            .init::<B>(&device)
            .load_record(loaded_model_record);

        experiment.fail("Hello")?;

        Ok(())
    }
}
