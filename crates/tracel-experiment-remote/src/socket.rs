use crossbeam::channel::{Receiver, RecvTimeoutError, Sender};
use std::num::NonZeroU64;
use std::{
    thread::JoinHandle,
    time::{Duration, Instant},
};
use tracel_client::{
    WebSocketClient,
    websocket::{ExperimentMessage, ServerMessage},
};
use tracel_experiment::{ActivityId, ExperimentRunControl};

#[derive(Debug, thiserror::Error)]
pub enum ThreadError {
    #[error("WebSocket error: {0}")]
    WebSocket(String),
    #[error("Unexpected panic in thread")]
    Panic,
}

const WEBSOCKET_CLOSE_ERROR: &str = "Failed to close WebSocket";

/// Well inside the idle timeout of the proxies between a run and the server: a run can go
/// minutes without a message (a kernel autotune, a cold compile), and a proxy that closes
/// the connection meanwhile ends the run on the server.
const LONGEST_SILENCE_BEFORE_A_KEEPALIVE_PING: Duration = Duration::from_secs(20);

/// Sends are written in order, so an acked flush probe proves everything
/// queued before it was written.
pub enum SocketCommand {
    Message(ExperimentMessage),
    Flush(Sender<()>),
}

#[derive(Debug)]
pub struct ThreadResult {}

struct ExperimentThread {
    ws_client: WebSocketClient,
    message_receiver: Receiver<SocketCommand>,
    control: ExperimentRunControl,
    last_frame_sent_at: Instant,
}

impl ExperimentThread {
    fn new(
        ws_client: WebSocketClient,
        message_receiver: Receiver<SocketCommand>,
        control: ExperimentRunControl,
    ) -> Self {
        Self {
            ws_client,
            message_receiver,
            control,
            last_frame_sent_at: Instant::now(),
        }
    }

    fn run(mut self) -> Result<ThreadResult, ThreadError> {
        let res = self.thread_loop();
        self.cleanup()?;
        res.map(|_| ThreadResult {})
    }

    fn cleanup(&mut self) -> Result<(), ThreadError> {
        self.ws_client
            .close()
            .map_err(|_| ThreadError::WebSocket(WEBSOCKET_CLOSE_ERROR.to_string()))?;
        self.ws_client
            .wait_until_closed()
            .map_err(|e| ThreadError::WebSocket(e.to_string()))?;
        Ok(())
    }

    fn handle_websocket_send<T: serde::Serialize + std::fmt::Debug>(
        &mut self,
        message: T,
    ) -> Result<(), ThreadError> {
        self.ws_client
            .send(message)
            .map_err(|e| ThreadError::WebSocket(e.to_string()))?;
        self.last_frame_sent_at = Instant::now();
        Ok(())
    }

    fn send_keepalive_ping_after_a_long_silence(&mut self) {
        if self.last_frame_sent_at.elapsed() < LONGEST_SILENCE_BEFORE_A_KEEPALIVE_PING {
            return;
        }
        if let Err(e) = self.ws_client.send_keepalive_ping() {
            tracing::warn!(error = ?e, "WebSocket keepalive ping failed");
        }
        self.last_frame_sent_at = Instant::now();
    }

    fn process_message(&mut self, command: SocketCommand) -> Result<(), ThreadError> {
        match command {
            SocketCommand::Message(message) => self.handle_websocket_send(message),
            SocketCommand::Flush(ack) => {
                let _ = ack.send(());
                Ok(())
            }
        }
    }

    fn thread_loop(&mut self) -> Result<(), ThreadError> {
        let poll = Duration::from_millis(50);

        loop {
            match self.ws_client.receive::<ServerMessage>() {
                Ok(Some(ServerMessage::CancelRequested)) => {
                    tracing::info!("Received server cancel request, triggering cancellation token");
                    self.control.cancel_run();
                }
                Ok(Some(ServerMessage::ActivityCancelRequested { id })) => {
                    let Some(id) = NonZeroU64::new(id).map(ActivityId::new) else {
                        tracing::warn!("Received activity cancellation request with id 0");
                        continue;
                    };

                    if self.control.cancel_activity(id) {
                        tracing::info!(
                            activity_id = id.as_u64(),
                            "Received activity cancel request"
                        );
                    } else {
                        tracing::warn!(
                            activity_id = id.as_u64(),
                            "Received cancel request for unknown or non-cancellable activity"
                        );
                    }
                }
                Ok(None) => {}
                Err(e) => tracing::error!(error = ?e, "WebSocket receive error"),
            }

            self.send_keepalive_ping_after_a_long_silence();

            match self.message_receiver.recv_timeout(poll) {
                Ok(message) => self.process_message(message)?,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }

        Ok(())
    }
}

pub struct ExperimentSocket {
    handle: JoinHandle<Result<ThreadResult, ThreadError>>,
}

impl ExperimentSocket {
    pub fn new(
        ws_client: WebSocketClient,
        message_receiver: Receiver<SocketCommand>,
        control: ExperimentRunControl,
    ) -> Self {
        let thread = ExperimentThread::new(ws_client, message_receiver, control);
        let handle = std::thread::spawn(move || thread.run());
        Self { handle }
    }

    pub fn join(self) -> Result<ThreadResult, ThreadError> {
        self.handle.join().unwrap_or(Err(ThreadError::Panic))
    }
}
