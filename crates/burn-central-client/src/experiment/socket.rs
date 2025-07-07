use std::{sync::mpsc, thread::JoinHandle};
use crate::experiment::log_store::TempLogStore;
use crate::websocket::WebSocketClient;

use super::ExperimentMessage;

#[derive(Debug)]
pub struct ThreadResult {}

struct ExperimentThread {
    ws_client: WebSocketClient,
    receiver: mpsc::Receiver<ExperimentMessage>,
    abort_receiver: mpsc::Receiver<()>,
    // State
    in_memory_logs: TempLogStore,
    iteration_count: usize,
}

impl ExperimentThread {
    pub fn new(
        ws_client: WebSocketClient,
        receiver: mpsc::Receiver<ExperimentMessage>,
        abort_receiver: mpsc::Receiver<()>,
        in_memory_logs: TempLogStore,
    ) -> Self {
        Self {
            ws_client,
            receiver,
            abort_receiver,
            in_memory_logs,
            iteration_count: 0,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ThreadError {
    #[error("WebSocket error: {0}")]
    WebSocket(String),
    #[error("Message channel closed unexpectedly")]
    MessageChannelClosed,
    #[error("Log storage failed: {0}")]
    LogFlushError(String),
    #[error("Failed to abort thread")]
    AbortError,
    #[error("Unexpected panic in thread")]
    Panic,
}

impl ExperimentThread {
    fn run(mut self) -> Result<ThreadResult, ThreadError> {
        self.thread_loop()?;
        self.ws_client
            .close()
            .map_err(|_| ThreadError::WebSocket("Failed to close WebSocket".to_string()))?;
        self.in_memory_logs
            .flush()
            .map_err(|e| ThreadError::LogFlushError(e.to_string()))?;

        Ok(ThreadResult {})
    }

    fn thread_loop(&mut self) -> Result<ThreadResult, ThreadError> {
        loop {
            match self.abort_receiver.try_recv() {
                Ok(_) => {
                    self.ws_client
                        .send(ExperimentMessage::Close)
                        .map_err(|e| ThreadError::WebSocket(e.to_string()))?;
                    return Ok(ThreadResult {});
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err(ThreadError::MessageChannelClosed);
                }
            }
            let res = self
                .receiver
                .recv()
                .map_err(|_| ThreadError::MessageChannelClosed)?;
            match res {
                ExperimentMessage::MetricLog {
                    name,
                    epoch,
                    iteration: _,
                    value,
                    group,
                } => {
                    self.iteration_count += 1;
                    self.ws_client
                        .send(ExperimentMessage::MetricLog {
                            name,
                            epoch,
                            iteration: self.iteration_count,
                            value,
                            group,
                        })
                        .map_err(|e| ThreadError::WebSocket(e.to_string()))?;
                }
                ExperimentMessage::Log(log) => {
                    self.in_memory_logs
                        .push(log.clone())
                        .map_err(|e| ThreadError::LogFlushError(e.to_string()))?;
                    self.ws_client
                        .send(ExperimentMessage::Log(log))
                        .map_err(|e| ThreadError::WebSocket(e.to_string()))?;
                }
                ExperimentMessage::Error(err) => {
                    self.ws_client
                        .send(ExperimentMessage::Error(err))
                        .map_err(|e| ThreadError::WebSocket(e.to_string()))?;
                }
                _ => {}
            }
        }
    }
}

#[derive(Debug)]
pub struct ExperimentSocket {
    abort_sender: mpsc::SyncSender<()>,
    handle: JoinHandle<Result<ThreadResult, ThreadError>>,
}

impl ExperimentSocket {
    pub fn new(
        ws_client: WebSocketClient,
        log_store: TempLogStore,
        receiver: mpsc::Receiver<ExperimentMessage>,
    ) -> Self {
        let (abort_sender, abort_receiver) = mpsc::sync_channel(1);

        let thread = ExperimentThread::new(ws_client, receiver, abort_receiver, log_store);
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
        self.handle
            .join()
            .unwrap_or_else(|_| Err(ThreadError::Panic))
    }
}
