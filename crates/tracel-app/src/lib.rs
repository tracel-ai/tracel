//! Runners for Tracel jobs: run registered experiments and inferences from a command line, or
//! serve them over HTTP.
//!
//! A job is registered from a capability job (an `ExperimentJob` or an `InferenceJob`) and a
//! [`Mapper`](mapper::Mapper) that decodes its input, which every runner hands over as JSON. Each
//! registered job has a [`JobDefinition`](tracel_job::JobDefinition): its name, kind, description
//! and input.
//!
//! - [`cli::Cli`] runs one job from the command line: `<job_name> [<input-json>] [<flags>]`, with
//!   a flag per field of the job's input, and returns the process's exit code: 0 when the job
//!   completed, 1 when it failed, 2 for an unknown job or an unusable flag or input, and 130 when
//!   it was cancelled. [`tracel_job::command`] builds that command line from the job definitions
//!   alone, so a program that reads a [`DefinitionsFile`](tracel_job::DefinitionsFile) builds the
//!   same one.
//! - `server::Server` serves every job over HTTP at `POST /{job_name}` (requires the `server`
//!   feature).
//!
//! An experiment records the input its mapper resolved, such as the input merged onto the
//! mapper's default, as its arguments.
//!
//! ## Describing jobs
//!
//! When `TRACEL_DESCRIBE` names a path, a runner writes a
//! [`DefinitionsFile`](tracel_job::DefinitionsFile) there, as JSON, instead of running a job, and
//! returns successfully. It writes `<path>.tmp` first and renames it to `<path>`, so the file is
//! never seen half written. While `TRACEL_DESCRIBE` is set, `ExperimentJob::run` returns an error
//! without creating an experiment.
//!
//! Each job's `input_example` comes from
//! [`JsonMapper::with_default`](mapper::JsonMapper::with_default), and its `input_schema` from
//! `JsonMapper::with_schema` (requires the `schema` feature).

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

pub use describe::DescribeError;
pub use job::{BoxError, DiscardOutput, IntoJob, Job, JobInput, JobOutput, PreparedJob};
pub use registry::JobRegistry;
