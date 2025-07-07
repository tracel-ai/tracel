use super::{ExperimentMessage, socket::ExperimentSocket};
use crate::experiment::error::ExperimentError;
use crate::experiment::log_store::TempLogStore;
use crate::experiment::socket::ThreadError;
use crate::http::EndExperimentStatus;
use crate::{
    ArtifactKind, ArtifactLoadArgs, ArtifactRecordArgs, ArtifactRecorder, http::HttpClient,
    schemas::ExperimentPath, websocket::WebSocketClient,
};
use burn::prelude::Backend;
use burn::record::{Record, Recorder};
use std::ops::Deref;
use std::sync::{Arc, Weak, mpsc};

/// Represents a handle to an experiment, allowing logging of artifacts, metrics, and messages.
#[derive(Clone, Debug)]
pub struct ExperimentHandle {
    recorder: Weak<ExperimentRecorder>,
}

impl ExperimentHandle {
    fn try_upgrade(&self) -> Result<Arc<ExperimentRecorder>, ExperimentError> {
        self.recorder
            .upgrade()
            .ok_or(ExperimentError::InactiveExperiment)
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
    ) -> Result<(), ExperimentError> {
        self.try_upgrade()?.log_artifact(name, kind, record)
    }

    /// Loads an artifact with the given name and device.
    pub fn load_artifact<B, R>(
        &self,
        name: impl Into<String>,
        device: &B::Device,
    ) -> Result<R, ExperimentError>
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
    ) -> Result<(), ExperimentError> {
        self.try_upgrade()?
            .log_metric(name, epoch, iteration, value, group)
    }

    /// Logs an info message.
    pub fn log_info(&self, message: impl Into<String>) {
        self.try_log_info(message)
            .expect("Failed to log info, experiment may have been closed or inactive");
    }

    /// Attempts to log an info message.
    pub fn try_log_info(&self, message: impl Into<String>) -> Result<(), ExperimentError> {
        self.try_upgrade()?.log_info(message)
    }

    /// Logs an error message.
    pub fn log_error(&self, error: impl Into<String>) {
        self.try_log_error(error)
            .expect("Failed to log error, experiment may have been closed or inactive");
    }

    /// Attempts to log an error message.
    pub fn try_log_error(&self, error: impl Into<String>) -> Result<(), ExperimentError> {
        self.try_upgrade()?.log_error(error)
    }
}

/// Represents a recorder for an experiment, allowing logging of artifacts, metrics, and messages.
/// It is used internally by the [Experiment](Experiment) struct to handle logging operations.
struct ExperimentRecorder {
    id: ExperimentPath,
    http_client: HttpClient,
    sender: mpsc::Sender<ExperimentMessage>,
}

impl ExperimentRecorder {
    pub fn id(&self) -> &ExperimentPath {
        &self.id
    }

    fn send(&self, message: ExperimentMessage) -> Result<(), ExperimentError> {
        self.sender
            .send(message)
            .map_err(|_| ExperimentError::SocketClosed)
    }

    pub fn log_artifact<B: Backend>(
        &self,
        name: impl Into<String>,
        kind: ArtifactKind,
        record: impl Record<B>,
    ) -> Result<(), ExperimentError> {
        let recorder = ArtifactRecorder::new(self.http_client.clone());
        let args = ArtifactRecordArgs {
            experiment_path: self.id.clone(),
            name: name.into(),
            kind,
        };
        recorder
            .record(record, args)
            .map_err(ExperimentError::BurnRecorderError)
    }

    pub fn load_artifact<B: Backend, R: Record<B>>(
        &self,
        name: impl Into<String>,
        device: &B::Device,
    ) -> Result<R, ExperimentError> {
        let recorder = ArtifactRecorder::new(self.http_client.clone());
        let args = ArtifactLoadArgs {
            experiment_path: self.id.clone(),
            name: name.into(),
        };
        recorder
            .load(args, device)
            .map_err(ExperimentError::BurnRecorderError)
    }

    pub fn log_metric(
        &self,
        name: impl Into<String>,
        epoch: usize,
        iteration: usize,
        value: f64,
        group: impl Into<String>,
    ) -> Result<(), ExperimentError> {
        let message = ExperimentMessage::MetricLog {
            name: name.into(),
            epoch,
            iteration,
            value,
            group: group.into(),
        };
        self.send(message)
    }

    pub fn log_info(&self, message: impl Into<String>) -> Result<(), ExperimentError> {
        self.send(ExperimentMessage::Log(message.into()))
    }

    pub fn log_error(&self, error: impl Into<String>) -> Result<(), ExperimentError> {
        self.send(ExperimentMessage::Error(error.into()))
    }
}

/// Represents an experiment in Burn Central, which is a run of a machine learning model or process.
pub struct Experiment {
    recorder: Arc<ExperimentRecorder>,
    socket: Option<ExperimentSocket>,
    // temporary field to allow dereferencing to handle
    _handle: ExperimentHandle,
}

impl Experiment {
    pub fn new(
        http_client: HttpClient,
        experiment_path: ExperimentPath,
    ) -> Result<Self, ExperimentError> {
        let mut ws_client = WebSocketClient::new();

        let ws_endpoint = http_client.format_websocket_url(
            &experiment_path.owner_name(),
            &experiment_path.project_name(),
            experiment_path.experiment_num(),
        );
        let cookie = http_client
            .get_session_cookie()
            .expect("Session cookie should be available");
        ws_client
            .connect(ws_endpoint, cookie)
            .map_err(ExperimentError::WebSocketError)?;

        let log_store = TempLogStore::new(http_client.clone(), experiment_path.clone());
        let (sender, receiver) = mpsc::channel();
        let socket = ExperimentSocket::new(ws_client, log_store, receiver);

        let recorder = Arc::new(ExperimentRecorder {
            id: experiment_path.clone(),
            http_client: http_client.clone(),
            sender,
        });

        let handle = ExperimentHandle {
            recorder: Arc::downgrade(&recorder),
        };

        Ok(Experiment {
            recorder,
            socket: Some(socket),
            _handle: handle,
        })
    }

    /// Returns a handle to the experiment, allowing logging of artifacts, metrics, and messages.
    pub fn handle(&self) -> ExperimentHandle {
        ExperimentHandle {
            recorder: Arc::downgrade(&self.recorder),
        }
    }

    fn finish_internal(&mut self, end_status: EndExperimentStatus) -> Result<(), ExperimentError> {
        let thread_result = match self.socket.take() {
            Some(socket) => socket.close(),
            None => return Err(ExperimentError::AlreadyFinished),
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
                return Err(ExperimentError::InternalError(
                    "Failed to abort thread.".into(),
                ));
            }
            Err(ThreadError::Panic) => {
                return Err(ExperimentError::InternalError(
                    "Experiment thread panicked".into(),
                ));
            }
        }

        if let Err(e) = self.recorder.send(ExperimentMessage::Close) {
            eprintln!("Warning: Failed to send close message: {}", e);
        }

        self.recorder
            .http_client
            .end_experiment(
                self.recorder.id.owner_name(),
                self.recorder.id.project_name(),
                self.recorder.id.experiment_num(),
                end_status,
            )
            .map_err(|e| {
                ExperimentError::InternalError(format!(
                    "Failed to end experiment: {}",
                    e.to_string()
                ))
            })?;

        Ok(())
    }

    pub fn finish(mut self) -> Result<(), ExperimentError> {
        self.finish_internal(EndExperimentStatus::Success)
    }

    pub fn fail(mut self, reason: impl Into<String>) -> Result<(), ExperimentError> {
        self.finish_internal(EndExperimentStatus::Fail(reason.into()))
    }
}

impl Drop for Experiment {
    fn drop(&mut self) {
        let _ = self.finish_internal(EndExperimentStatus::Fail(
            "Experiment dropped without finishing".to_string(),
        ));
    }
}

/// Temporary implementation to allow dereferencing the Experiment to its recorder
/// This will be removed once the experiment logging api is completed
impl Deref for Experiment {
    type Target = ExperimentHandle;

    fn deref(&self) -> &Self::Target {
        &self._handle
    }
}

#[cfg(test)]
mod test {
    use crate::ArtifactKind;
    use crate::experiment::Experiment;
    use crate::http::HttpClient;
    use crate::schemas::ExperimentPath;
    use burn::backend::NdArray;
    use burn::nn::conv::{Conv2d, Conv2dConfig};
    use burn::nn::pool::{AdaptiveAvgPool2d, AdaptiveAvgPool2dConfig};
    use burn::nn::{Dropout, DropoutConfig, Linear, LinearConfig, Relu};
    use burn::prelude::*;
    use burn::record::Recorder;

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

    #[test]
    fn test_experiment_handle() -> anyhow::Result<()> {
        type TestBackend = NdArray;
        type TestDevice = <TestBackend as Backend>::Device;

        let device = TestDevice::default();
        train_experiment::<TestBackend>(device)?;

        Ok(())
    }

    fn train_experiment<B: Backend>(device: B::Device) -> anyhow::Result<()> {
        let experiment = Experiment::new(
            HttpClient::new_without_credentials("http://localhost:9001".parse().unwrap()),
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
        let experiment = Experiment::new(
            HttpClient::new_without_credentials("http://localhost:8000".parse().unwrap()),
            ExperimentPath::try_from("test_owner/test_project/1".to_string()).unwrap(),
        )?;

        let model_config = ModelConfig::new(10, 128);

        // load the artifact back
        let loaded_model_record = experiment
            .load_artifact("model", &device)
            .expect("Failed to load model artifact");

        let model = model_config
            .init::<B>(&device)
            .load_record(loaded_model_record);

        experiment.fail("Hello")?;

        Ok(())
    }
}
