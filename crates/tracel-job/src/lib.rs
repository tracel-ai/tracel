#![deny(missing_docs)]

//! Tracel jobs run as processes: the protocol between a job binary and the programs that launch
//! it, such as `tracel run` and compute providers, and, with the `cli` feature, the command line
//! of its jobs.
//!
//! The protocol says how a launcher asks the binary for its jobs, how it runs one, and what the
//! binary's exit code says. Jobs served another way, such as by a Station runner, follow that
//! transport's own API. It is not an API for writing jobs.
//!
//! It holds the protocol's shapes and names, and does no I/O: [`DefinitionsFile::to_json`] gives
//! the exact text of the definitions file, and [`DefinitionsFile::from_json`] reads it back,
//! checking its [`PROTOCOL`]. Writing and reading the file is the caller's.
//!
//! ## Describing the jobs
//!
//! When [`TRACEL_DESCRIBE`] names a path, the binary writes its [`DefinitionsFile`] there instead
//! of running a job, and exits with code 0. It writes the file to `<path>.tmp` first, then renames
//! it to `<path>`, so it is never read half written:
//!
//! ```json
//! {
//!   "protocol": 1,
//!   "sdk_version": "0.10.0",
//!   "jobs": [
//!     {
//!       "name": "train",
//!       "description": "Train the model",
//!       "input_schema": null,
//!       "input_example": { "epochs": 10, "optimizer": { "lr": 0.001 } }
//!     }
//!   ]
//! }
//! ```
//!
//! `jobs` is ordered by name. `input_example` is the job's default input, which its input is
//! merged onto, and `input_schema` the JSON Schema of its input, when the binary gives one.
//!
//! ## Running a job
//!
//! `<binary> <job> [<input-json>]` runs the job `<job>`, with the JSON document merged onto the
//! job's default input; without one, the job runs with its default input. [`TRACEL_JOB_NUM`]
//! gives the job the number its launcher gave it.
//!
//! The binary's [`ExitCode`] says how the job ended:
//!
//! | Exit code | Meaning |
//! | --- | --- |
//! | 0, [`Completed`](ExitCode::Completed) | The job completed, the help, version or completion script was printed, or the definitions file was written |
//! | 1, [`Failed`](ExitCode::Failed) | The job failed, or the definitions file could not be written |
//! | 2, [`Rejected`](ExitCode::Rejected) | No job or an unknown job is named, and stderr lists the job names; or a flag or the input is unusable or does not decode |

#[cfg(feature = "cli")]
pub mod cli;
mod definitions;
mod run;

pub use definitions::{DefinitionsFile, JobDefinition, ParseError, TRACEL_DESCRIBE};
pub use run::{ExitCode, TRACEL_JOB_NUM};

/// The protocol version a [`DefinitionsFile`] follows, which it gives as `protocol`.
pub const PROTOCOL: u32 = 1;
