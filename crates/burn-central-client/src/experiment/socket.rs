use crate::api::ClientError;
use crate::experiment::log_store::TempLogStore;
use crate::experiment::message::ExperimentMessage;
use crate::websocket::WebSocketClient;
use crossbeam::channel::{Receiver, Sender, select};
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
const CHANNEL_BUFFER_SIZE: usize = 1;

#[derive(Debug)]
pub struct ThreadResult {}

struct ExperimentThread {
    ws_client: WebSocketClient,
    message_receiver: Receiver<ExperimentMessage>,
    abort_signal: Receiver<()>,
    log_store: TempLogStore,
    iteration_count: usize,
}

impl ExperimentThread {
    pub fn new(
        ws_client: WebSocketClient,
        message_receiver: Receiver<ExperimentMessage>,
        abort_signal: Receiver<()>,
        log_store: TempLogStore,
    ) -> Self {
        Self {
            ws_client,
            message_receiver,
            abort_signal,
            log_store,
            iteration_count: 0,
        }
    }

    fn run(mut self) -> Result<ThreadResult, ThreadError> {
        self.thread_loop()?;
        self.cleanup()?;
        Ok(ThreadResult {})
    }

    fn cleanup(&mut self) -> Result<(), ThreadError> {
        self.ws_client
            .close()
            .map_err(|_| ThreadError::WebSocket(WEBSOCKET_CLOSE_ERROR.to_string()))?;
        self.log_store.flush().map_err(ThreadError::LogFlushError)?;
        Ok(())
    }

    fn handle_websocket_send<T: serde::Serialize>(
        &mut self,
        message: T,
    ) -> Result<(), ThreadError> {
        self.ws_client
            .send(message)
            .map_err(|e| ThreadError::WebSocket(e.to_string()))
    }

    fn handle_metric_log(
        &mut self,
        name: String,
        epoch: usize,
        value: f64,
        group: String,
    ) -> Result<(), ThreadError> {
        self.iteration_count += 1;
        self.handle_websocket_send(ExperimentMessage::MetricLog {
            name,
            epoch,
            iteration: self.iteration_count,
            value,
            group,
        })
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
                        ExperimentMessage::MetricLog { name, epoch, iteration: _, value, group } => {
                            self.handle_metric_log(name, epoch, value, group)?;
                        }
                        ExperimentMessage::Log(log) => {
                            self.handle_log_message(log)?;
                        }
                        ExperimentMessage::InputUsed(input) => {
                            self.handle_websocket_send(ExperimentMessage::InputUsed(input))?;
                        }
                        ExperimentMessage::Error(err) => {
                            self.handle_websocket_send(ExperimentMessage::Error(err))?;
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
        let (abort_sender, abort_signal) = crossbeam::channel::bounded(CHANNEL_BUFFER_SIZE);
        let thread = ExperimentThread::new(ws_client, message_receiver, abort_signal, log_store);
        let handle = std::thread::spawn(|| thread.run());
        Self {
            abort_sender,
            handle,
        }
    }

    pub fn close(self) -> Result<ThreadResult, ThreadError> {
        println!("Closing experiment socket");
        self.abort_sender
            .send(())
            .map_err(|_| ThreadError::AbortError)?;
        self.handle.join().unwrap_or(Err(ThreadError::Panic))
    }
}
