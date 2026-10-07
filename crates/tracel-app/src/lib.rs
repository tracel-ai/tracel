//! Runners for Tracel jobs: run registered experiments and inferences from a command line, or
//! serve them over HTTP.
//!
//! A job is registered from a capability job (an `ExperimentJob` or an `InferenceJob`) and a
//! [`Mapper`](mapper::Mapper) that decodes its input, which every runner hands over as JSON. Each
//! registered job has a [`JobDefinition`]: its name, kind, description and input.
//!
//! - [`cli::Cli`] runs one job from the command line: `<job_name> [<input-json>]`.
//! - `server::Server` serves every job over HTTP at `POST /{job_name}` (requires the `server`
//!   feature).
//!
//! ## Describing jobs
//!
//! When `TRACEL_DESCRIBE` names a path, a runner writes a [`DefinitionsFile`] there, as JSON,
//! instead of running a job, and returns `Ok(())`. It writes `<path>.tmp` first and renames it to
//! `<path>`, so the file is never seen half written. While `TRACEL_DESCRIBE` is set,
//! `ExperimentJob::run` returns an error without creating an experiment.
//!
//! ```json
//! {
//!   "protocol": 1,
//!   "sdk_version": "0.10.0",
//!   "runner": "cli",
//!   "jobs": [
//!     {
//!       "name": "train",
//!       "kind": "experiment",
//!       "description": "Train the model",
//!       "input_schema": null,
//!       "input_example": { "epochs": 10, "optimizer": { "lr": 0.001 } }
//!     }
//!   ]
//! }
//! ```
//!
//! `input_example` comes from [`JsonMapper::with_default`](mapper::JsonMapper::with_default), and
//! `input_schema` from `JsonMapper::with_schema` (requires the `schema` feature).

mod adapter;
/// Command-line runner.
pub mod cli;
mod describe;
mod job;
pub mod mapper;
mod registry;
/// HTTP server runner.
#[cfg(feature = "server")]
pub mod server;
#[cfg(test)]
mod test_support;

pub use describe::{DefinitionsFile, DescribeError};
pub use job::{
    BoxError, DiscardOutput, IntoJob, Job, JobDefinition, JobInput, JobKind, JobOutput, PreparedJob,
};
pub use registry::JobRegistry;
