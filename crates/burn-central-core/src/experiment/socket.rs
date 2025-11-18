use crate::experiment::log_store::TempLogStore;
use burn_central_client::{ClientError, WebSocketClient, websocket::ExperimentMessage};
use crossbeam::channel::{Receiver, Sender, select};
use derive_new::new;
use std::thread::JoinHandle;

#[derive(Debug, thiserror::Error)]
pub enum ThreadError {
    #[error("WebSocket error: {0}")]
    WebSocket(String),
    #[error("Message channel closed unexpectedly")]
    MessageChannelClosed,
    #[error("Log storage failed: {0}")]
    LogFlushError(ClientError),
    #[error("Failed to abort thread")]
    AbortError,
    #[error("Unexpected panic in thread")]
    Panic,
}

const WEBSOCKET_CLOSE_ERROR: &str = "Failed to close WebSocket";

#[derive(Debug)]
pub struct ThreadResult {}

#[derive(new)]
struct ExperimentThread {
    ws_client: WebSocketClient,
    message_receiver: Receiver<ExperimentMessage>,
    abort_signal: Receiver<()>,
    log_store: TempLogStore,
}

impl ExperimentThread {
    fn run(mut self) -> Result<ThreadResult, ThreadError> {
        let res = self.thread_loop();
        self.cleanup()?;
        res.map(|_| ThreadResult {})
    }

    fn cleanup(&mut self) -> Result<(), ThreadError> {
        self.ws_client
            .close()
            .map_err(|_| ThreadError::WebSocket(WEBSOCKET_CLOSE_ERROR.to_string()))?;
        self.log_store.flush().map_err(ThreadError::LogFlushError)?;
        Ok(())
    }

    fn handle_websocket_send<T: serde::Serialize + std::fmt::Debug>(
        &mut self,
        message: T,
    ) -> Result<(), ThreadError> {
        self.ws_client
            .send(message)
            .map_err(|e| ThreadError::WebSocket(e.to_string()))
    }

    fn handle_log_message(&mut self, log: String) -> Result<(), ThreadError> {
        self.log_store
            .push(log.clone())
            .map_err(ThreadError::LogFlushError)?;
        self.handle_websocket_send(ExperimentMessage::Log(log))
    }

    fn thread_loop(&mut self) -> Result<(), ThreadError> {
        loop {
            select! {
                recv(self.abort_signal) -> _ => {
                    return Ok(());
                }
                recv(self.message_receiver) -> msg => {
                    let message = msg.map_err(|_| ThreadError::MessageChannelClosed)?;
                    match message {
                        ExperimentMessage::MetricsLog { .. } => {
                            self.handle_websocket_send(message)?;
                        }
                        ExperimentMessage::MetricDefinitionLog { .. } => {
                            self.handle_websocket_send(message)?;
                        }
                        ExperimentMessage::Log(log) => {
                            self.handle_log_message(log)?;
                        }
                        ExperimentMessage::EpochSummaryLog { .. } => {
                            self.handle_websocket_send(message)?;
                        }
                        value => {
                            self.handle_websocket_send(value)?;
                        }
                    }
                }
            }
        }
    }
}

#[derive(Debug)]
pub struct ExperimentSocket {
    abort_sender: Sender<()>,
    handle: JoinHandle<Result<ThreadResult, ThreadError>>,
}

impl ExperimentSocket {
    pub fn new(
        ws_client: WebSocketClient,
        log_store: TempLogStore,
        message_receiver: Receiver<ExperimentMessage>,
    ) -> Self {
        let (abort_sender, abort_signal) = crossbeam::channel::bounded(1);
        let thread = ExperimentThread::new(ws_client, message_receiver, abort_signal, log_store);
        let handle = std::thread::spawn(|| thread.run());
        Self {
            abort_sender,
            handle,
        }
    }

    pub fn close(self) -> Result<ThreadResult, ThreadError> {
        self.abort_sender
            .send(())
            .map_err(|_| ThreadError::AbortError)?;
        self.handle.join().unwrap_or(Err(ThreadError::Panic))
    }
}
