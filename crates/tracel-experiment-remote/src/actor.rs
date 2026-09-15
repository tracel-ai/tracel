use std::future::Future;
use std::num::NonZeroU64;
use std::pin::pin;

use async_channel::{Receiver, Sender};
use futures::channel::oneshot;
use futures::future::{Either, select};
use tracel_client::WebSocketClient;
use tracel_client::websocket::{ExperimentMessage, ServerMessage, WebSocketError};
use tracel_experiment::{ActivityId, ExperimentRunControl};
use tracel_task::{Job, MaybeSend};

/// Why the experiment socket could not carry out an operation.
#[derive(Debug, thiserror::Error)]
pub enum SocketError {
    /// The transport failed.
    #[error("WebSocket error: {0}")]
    WebSocket(String),
    /// The actor driving the socket stopped before it could answer.
    #[error("the experiment socket is closed")]
    Closed,
}

impl From<WebSocketError> for SocketError {
    fn from(error: WebSocketError) -> Self {
        Self::WebSocket(error.to_string())
    }
}

/// The actor driving the socket has stopped, so nothing more can be queued for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the experiment socket is closed")]
pub struct SocketClosed;

/// The connection an experiment run speaks the remote protocol over.
///
/// One actor owns the socket and is the only thing that touches it, so an implementation never
/// sees two operations at once.
pub trait ExperimentSocket: MaybeSend + 'static {
    /// Sends `message` to the server.
    fn send(
        &mut self,
        message: ExperimentMessage,
    ) -> impl Future<Output = Result<(), SocketError>> + MaybeSend;

    /// Waits for the next message from the server; `None` once the peer has closed the
    /// connection.
    fn next(
        &mut self,
    ) -> impl Future<Output = Result<Option<ServerMessage>, SocketError>> + MaybeSend;

    /// Closes the connection, waiting for the closing handshake.
    fn close(&mut self) -> impl Future<Output = Result<(), SocketError>> + MaybeSend;
}

impl ExperimentSocket for WebSocketClient {
    async fn send(&mut self, message: ExperimentMessage) -> Result<(), SocketError> {
        WebSocketClient::send(self, message)
            .await
            .map_err(SocketError::from)
    }

    async fn next(&mut self) -> Result<Option<ServerMessage>, SocketError> {
        WebSocketClient::next(self).await.map_err(SocketError::from)
    }

    async fn close(&mut self) -> Result<(), SocketError> {
        WebSocketClient::close(self)
            .await
            .map_err(SocketError::from)
    }
}

/// A run's end of the actor that drives its [`ExperimentSocket`].
///
/// Messages are queued without waiting and written in the order they were queued, so a
/// [`flush`](SocketHandle::flush) or [`close`](SocketHandle::close) covers everything queued
/// before it. Clones share the queue.
#[derive(Clone)]
pub struct SocketHandle {
    mailbox: Sender<Command>,
}

impl SocketHandle {
    /// Creates the handle and the actor loop that owns `socket`, writes what the handle queues,
    /// and applies the server's cancellation requests to `control`. The caller runs the loop
    /// wherever its loops run.
    ///
    /// The loop ends once the socket is closed, the peer hangs up, a write fails, or every
    /// handle is dropped.
    pub fn start<S: ExperimentSocket>(
        socket: S,
        control: ExperimentRunControl,
    ) -> (Self, impl Future<Output = ()> + MaybeSend + 'static) {
        let (mailbox, commands) = async_channel::unbounded();
        (Self { mailbox }, run(socket, control, commands))
    }

    /// Queues `message` for the socket without waiting.
    pub fn send(&self, message: ExperimentMessage) -> Result<(), SocketClosed> {
        self.mailbox
            .try_send(Command::Send(message))
            .map_err(|_| SocketClosed)
    }

    /// Resolves once every message queued before it has been written to the socket.
    pub fn flush(&self) -> Job<(), SocketError> {
        self.ask(Command::Flush)
    }

    /// Closes the socket once every message queued before it has been written, and stops the
    /// actor.
    pub fn close(&self) -> Job<(), SocketError> {
        self.ask(Command::Close)
    }

    /// Queues a command carrying a reply slot; an actor that stops first answers `Closed`.
    fn ask(&self, command: fn(Reply) -> Command) -> Job<(), SocketError> {
        let (reply, answer) = oneshot::channel();
        match self.mailbox.try_send(command(reply)) {
            Ok(()) => Job::new(async move { answer.await.unwrap_or(Err(SocketError::Closed)) }),
            Err(_) => Job::failed(SocketError::Closed),
        }
    }
}

type Reply = oneshot::Sender<Result<(), SocketError>>;

enum Command {
    Send(ExperimentMessage),
    Flush(Reply),
    Close(Reply),
}

impl Command {
    fn reject(self) {
        match self {
            Command::Send(_) => {}
            Command::Flush(reply) | Command::Close(reply) => {
                let _ = reply.send(Err(SocketError::Closed));
            }
        }
    }
}

enum Step {
    Received(Result<Option<ServerMessage>, SocketError>),
    Command(Option<Command>),
}

async fn run<S: ExperimentSocket>(
    mut socket: S,
    control: ExperimentRunControl,
    commands: Receiver<Command>,
) {
    if let Err(error) = drive(&mut socket, &control, &commands).await {
        tracing::warn!(error = %error, "The experiment socket stopped");
    }

    commands.close();
    while let Ok(command) = commands.try_recv() {
        command.reject();
    }
}

async fn drive<S: ExperimentSocket>(
    socket: &mut S,
    control: &ExperimentRunControl,
    commands: &Receiver<Command>,
) -> Result<(), SocketError> {
    loop {
        let step = {
            let received = pin!(socket.next());
            let command = pin!(commands.recv());
            match select(received, command).await {
                Either::Left((received, _)) => Step::Received(received),
                Either::Right((command, _)) => Step::Command(command.ok()),
            }
        };

        match step {
            Step::Received(Ok(Some(message))) => apply(control, message),
            Step::Received(Ok(None)) => return Ok(()),
            Step::Received(Err(error)) => {
                tracing::error!(error = ?error, "WebSocket receive error");
            }
            Step::Command(Some(Command::Send(message))) => socket.send(message).await?,
            Step::Command(Some(Command::Flush(reply))) => {
                let _ = reply.send(Ok(()));
            }
            Step::Command(Some(Command::Close(reply))) => {
                let _ = reply.send(socket.close().await);
                return Ok(());
            }
            Step::Command(None) => return socket.close().await,
        }
    }
}

fn apply(control: &ExperimentRunControl, message: ServerMessage) {
    match message {
        ServerMessage::CancelRequested => {
            tracing::info!("Received server cancel request, triggering cancellation token");
            control.cancel_run();
        }
        ServerMessage::ActivityCancelRequested { id } => {
            let Some(id) = NonZeroU64::new(id).map(ActivityId::new) else {
                tracing::warn!("Received activity cancellation request with id 0");
                return;
            };

            if control.cancel_activity(id) {
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
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use futures::channel::oneshot;
    use tracel_artifact::bundle::FsBundle;
    use tracel_experiment::error::ExperimentError;
    use tracel_experiment::reader::{
        ExperimentArtifactReader, ExperimentReaderError, LoadedArtifact,
    };
    use tracel_experiment::session::{Event, ExperimentCompletion, ExperimentSession};
    use tracel_experiment::{ArtifactKind, ExperimentId, ExperimentRun};

    use super::*;

    /// Starts the actor with its loop driven on a thread of its own.
    fn started<S: ExperimentSocket>(socket: S, control: ExperimentRunControl) -> SocketHandle {
        let (handle, run) = SocketHandle::start(socket, control);
        std::thread::spawn(move || futures::executor::block_on(run));
        handle
    }

    /// A socket whose peer is the test.
    struct FakeSocket {
        incoming: Receiver<ServerMessage>,
        state: Arc<PeerState>,
        close_gate: Option<oneshot::Receiver<()>>,
    }

    #[derive(Default)]
    struct PeerState {
        sent: Mutex<Vec<ExperimentMessage>>,
        closing: AtomicBool,
        closed: AtomicBool,
        released: AtomicBool,
    }

    /// The test's end of a [`FakeSocket`].
    struct Peer {
        incoming: Sender<ServerMessage>,
        state: Arc<PeerState>,
    }

    impl Peer {
        fn push(&self, message: ServerMessage) {
            self.incoming.try_send(message).unwrap();
        }

        fn hang_up(&self) {
            self.incoming.close();
        }

        fn sent_names(&self) -> Vec<String> {
            self.state
                .sent
                .lock()
                .unwrap()
                .iter()
                .map(|message| match message {
                    ExperimentMessage::MetricDefinitionLog { name, .. } => name.clone(),
                    other => panic!("unexpected message {other:?}"),
                })
                .collect()
        }

        fn closing(&self) -> bool {
            self.state.closing.load(Ordering::SeqCst)
        }

        fn closed(&self) -> bool {
            self.state.closed.load(Ordering::SeqCst)
        }

        fn released(&self) -> bool {
            self.state.released.load(Ordering::SeqCst)
        }
    }

    impl ExperimentSocket for FakeSocket {
        async fn send(&mut self, message: ExperimentMessage) -> Result<(), SocketError> {
            self.state.sent.lock().unwrap().push(message);
            Ok(())
        }

        async fn next(&mut self) -> Result<Option<ServerMessage>, SocketError> {
            Ok(self.incoming.recv().await.ok())
        }

        async fn close(&mut self) -> Result<(), SocketError> {
            self.state.closing.store(true, Ordering::SeqCst);
            if let Some(gate) = self.close_gate.take() {
                let _ = gate.await;
            }
            self.state.closed.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    impl Drop for FakeSocket {
        fn drop(&mut self) {
            self.state.released.store(true, Ordering::SeqCst);
        }
    }

    fn fake_socket(close_gate: Option<oneshot::Receiver<()>>) -> (FakeSocket, Peer) {
        let (sender, receiver) = async_channel::unbounded();
        let state = Arc::new(PeerState::default());
        let socket = FakeSocket {
            incoming: receiver,
            state: Arc::clone(&state),
            close_gate,
        };
        let peer = Peer {
            incoming: sender,
            state,
        };
        (socket, peer)
    }

    fn definition(name: &str) -> ExperimentMessage {
        ExperimentMessage::MetricDefinitionLog {
            name: name.to_string(),
            description: None,
            unit: None,
            higher_is_better: true,
        }
    }

    fn holds_within(secs: u64, condition: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < deadline {
            if condition() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    struct NullSession;

    impl ExperimentSession for NullSession {
        fn record_event(&self, _event: Event) -> Result<(), ExperimentError> {
            Ok(())
        }

        fn save_artifact(
            &self,
            _name: String,
            _kind: ArtifactKind,
            _bundle: FsBundle,
        ) -> Job<(), ExperimentError> {
            Job::ready(())
        }

        fn finish(&self, _completion: ExperimentCompletion) -> Job<(), ExperimentError> {
            Job::ready(())
        }
    }

    struct NullReader;

    impl ExperimentArtifactReader for NullReader {
        fn load_artifact_raw(
            &self,
            _experiment_id: ExperimentId,
            _name: String,
        ) -> Job<LoadedArtifact, ExperimentReaderError> {
            Job::failed(ExperimentReaderError::new("no artifacts"))
        }
    }

    #[test]
    fn queued_messages_reach_the_socket_in_order() {
        let (socket, peer) = fake_socket(None);
        let handle = started(socket, ExperimentRunControl::default());

        handle.send(definition("one")).unwrap();
        handle.send(definition("two")).unwrap();
        handle.send(definition("three")).unwrap();
        handle.flush().block().unwrap();

        assert_eq!(peer.sent_names(), ["one", "two", "three"]);
    }

    #[test]
    fn everything_queued_before_close_is_written_before_the_socket_closes() {
        let (socket, peer) = fake_socket(None);
        let handle = started(socket, ExperimentRunControl::default());

        handle.send(definition("one")).unwrap();
        handle.send(definition("two")).unwrap();
        handle.close().block().unwrap();

        assert_eq!(peer.sent_names(), ["one", "two"]);
        assert!(peer.closed());
    }

    #[test]
    fn a_server_cancel_request_cancels_the_run() {
        let (socket, peer) = fake_socket(None);
        let control = ExperimentRunControl::default();
        let _handle = started(socket, control.clone());

        peer.push(ServerMessage::CancelRequested);

        assert!(holds_within(5, || control.is_run_cancelled()));
    }

    #[test]
    fn an_activity_cancel_request_with_a_known_id_cancels_that_activity() {
        let (socket, peer) = fake_socket(None);
        let control = ExperimentRunControl::default();
        let run =
            ExperimentRun::new_with_control("remote/1", NullSession, NullReader, control.clone());
        let activity = run.activity("node").cancellable().start();
        let _handle = started(socket, control.clone());

        peer.push(ServerMessage::ActivityCancelRequested {
            id: activity.id().as_u64(),
        });

        assert!(holds_within(5, || activity.is_cancel_requested()));
        assert!(!control.is_run_cancelled());
    }

    #[test]
    fn close_resolves_only_once_the_socket_has_closed() {
        let (open, gate) = oneshot::channel();
        let (socket, peer) = fake_socket(Some(gate));
        let handle = started(socket, ExperimentRunControl::default());

        let mut closing = handle.close();
        assert!(holds_within(5, || peer.closing()));
        assert!(closing.try_poll().is_none());

        open.send(()).unwrap();

        closing.block().unwrap();
        assert!(peer.closed());
    }

    #[test]
    fn the_actor_stops_when_the_peer_hangs_up() {
        let (socket, peer) = fake_socket(None);
        let handle = started(socket, ExperimentRunControl::default());

        peer.hang_up();

        assert!(holds_within(5, || peer.released()));
        assert_eq!(handle.send(definition("late")), Err(SocketClosed));
        assert!(matches!(handle.close().block(), Err(SocketError::Closed)));
    }
}
