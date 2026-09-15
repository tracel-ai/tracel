#![deny(missing_docs)]

//! Websocket-backed [`ExperimentSession`](tracel_experiment::session::ExperimentSession) shared by
//! every backend that speaks the Tracel remote experiment protocol.

mod actor;
// The session still meets the synchronous `ExperimentSession` contract at a native edge.
#[cfg(not(target_arch = "wasm32"))]
mod session;

pub use actor::{ExperimentSocket, SocketClosed, SocketError, SocketHandle};
#[cfg(not(target_arch = "wasm32"))]
pub use session::{
    ArtifactUploadError, ArtifactUploader, BoxedArtifactUploader, RemoteExperimentSession,
};
