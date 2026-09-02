#![deny(missing_docs)]

//! Websocket-backed [`ExperimentSession`](tracel_experiment::session::ExperimentSession) shared by
//! every backend that speaks the Tracel remote experiment protocol.

mod session;
mod socket;

pub use session::{
    ArtifactUploadError, ArtifactUploader, BoxedArtifactUploader, RemoteExperimentSession,
};
