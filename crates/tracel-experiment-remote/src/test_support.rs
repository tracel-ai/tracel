use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_channel::{Receiver, Sender};
use futures::channel::oneshot;
use tracel_client::websocket::{ExperimentMessage, ServerMessage};
use tracel_experiment::ExperimentRunControl;

use crate::actor::{ExperimentSocket, SocketError, SocketHandle};

/// Starts the actor with its loop driven on a thread of its own.
pub(crate) fn started<S: ExperimentSocket>(
    socket: S,
    control: ExperimentRunControl,
) -> SocketHandle {
    let (handle, run) = SocketHandle::start(socket, control);
    std::thread::spawn(move || futures::executor::block_on(run));
    handle
}

/// A socket whose peer is the test.
pub(crate) struct FakeSocket {
    incoming: Receiver<ServerMessage>,
    state: Arc<PeerState>,
    close_gate: Option<oneshot::Receiver<()>>,
}

#[derive(Default)]
struct PeerState {
    sent: Mutex<Vec<String>>,
    closing: AtomicBool,
    closed: AtomicBool,
    released: AtomicBool,
}

/// The test's end of a [`FakeSocket`].
pub(crate) struct Peer {
    incoming: Sender<ServerMessage>,
    state: Arc<PeerState>,
}

impl Peer {
    pub(crate) fn push(&self, message: ServerMessage) {
        self.incoming.try_send(message).unwrap();
    }

    pub(crate) fn hang_up(&self) {
        self.incoming.close();
    }

    /// What the socket has been asked to write, one label per message, in order.
    pub(crate) fn sent(&self) -> Vec<String> {
        self.state.sent.lock().unwrap().clone()
    }

    pub(crate) fn closing(&self) -> bool {
        self.state.closing.load(Ordering::SeqCst)
    }

    pub(crate) fn closed(&self) -> bool {
        self.state.closed.load(Ordering::SeqCst)
    }

    pub(crate) fn released(&self) -> bool {
        self.state.released.load(Ordering::SeqCst)
    }
}

impl ExperimentSocket for FakeSocket {
    async fn send(&mut self, message: ExperimentMessage) -> Result<(), SocketError> {
        self.state.sent.lock().unwrap().push(label(&message));
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

/// A metric definition reads as its name, a completion as `complete:<completion>`.
fn label(message: &ExperimentMessage) -> String {
    match message {
        ExperimentMessage::MetricDefinitionLog { name, .. } => name.clone(),
        ExperimentMessage::ExperimentComplete(completion) => format!("complete:{completion:?}"),
        other => panic!("unexpected message {other:?}"),
    }
}

pub(crate) fn fake_socket(close_gate: Option<oneshot::Receiver<()>>) -> (FakeSocket, Peer) {
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

pub(crate) fn definition(name: &str) -> ExperimentMessage {
    ExperimentMessage::MetricDefinitionLog {
        name: name.to_string(),
        description: None,
        unit: None,
        higher_is_better: true,
    }
}

pub(crate) fn holds_within(secs: u64, condition: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}
