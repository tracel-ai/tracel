#![deny(missing_docs)]

//! The contract between the process adapter of the Tracel runtime and the programs that launch job
//! binaries, such as `tracel run` and compute providers: how a launcher asks a job binary for its
//! jobs, how it runs one, and what the binary's exit code says. It is not an API for writing jobs.
//!
//! It holds the contract's shapes and names, and does no I/O: [`DefinitionsFile::to_json`] gives
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
//! job's default input; without one, the job runs with its default input.
//!
//! When [`TRACEL_JOB_NUM`] gives the job a number, each experiment the job starts records it as
//! its [`JOB_NUM_ATTRIBUTE`], `tracel.job_num`, a number when it is one, which links the
//! experiment back to the job.
//!
//! The binary's exit code says how the job ended:
//!
//! | Exit code | Meaning |
//! | --- | --- |
//! | 0 | The job completed, the help, version or completion script was printed, or the definitions file was written |
//! | 1 | The job failed, or the definitions file could not be written |
//! | 2 | No job or an unknown job is named, and stderr lists the job names; or a flag or the input is unusable or does not decode |
//! | 130 | The job was asked to stop |
//!
//! SIGTERM, SIGINT or SIGHUP, or Ctrl-C on Windows, asks the running job to stop. The job ends by
//! what it returns, and the binary exits with code 130. A second signal ends the binary at once. A
//! launcher sends SIGKILL after a grace period; `tracel run` waits 30 seconds.

mod definitions;
mod job_num;

pub use definitions::{DefinitionsFile, JobDefinition, ParseError, TRACEL_DESCRIBE};
pub use job_num::{JOB_NUM_ATTRIBUTE, TRACEL_JOB_NUM};

/// The protocol version a [`DefinitionsFile`] follows, which it gives as `protocol`.
pub const PROTOCOL: u32 = 1;
