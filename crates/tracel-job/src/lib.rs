#![deny(missing_docs)]

//! What describes a program's jobs from outside the program that runs them: the files it writes
//! for the program that launches it, and its command line.
//!
//! - [`DefinitionsFile`]: the jobs a program can run, each given by its [`JobDefinition`]. When
//!   [`TRACEL_DESCRIBE`] names a path, the program writes the file there instead of running a
//!   job.
//! - [`RunReport`]: the experiment a run created, and how the run went. When
//!   [`TRACEL_REPORT_FILE`] names a path, an experiment run writes its report there when it
//!   creates its experiment, and again when it ends.
//! - [`command`]: the command line of a program's jobs, built from their definitions alone, so a
//!   program that reads a definitions file builds the same command line as the program that wrote
//!   it. [`job_input`] reads a job's input from its arguments.
//!
//! Both files are JSON and give the [`PROTOCOL`] version they follow. Each is written to
//! `<path>.tmp` first, then renamed to `<path>`, so it is never read half written.
//!
//! A definitions file:
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
//! A run report:
//!
//! ```json
//! {
//!   "protocol": 1,
//!   "job": "train",
//!   "experiment": { "num": 42, "url": "https://console.tracel.ai/users/me/projects/demo/experiments/42" },
//!   "status": "completed",
//!   "started_at": "2026-10-06T14:02:11Z",
//!   "finished_at": "2026-10-06T14:31:40Z",
//!   "error": null
//! }
//! ```

mod command;
mod definitions;
mod flags;
mod json_file;
mod report;

pub use command::{command, completions, job_command, job_input};
pub use definitions::{DefinitionsFile, JobDefinition, JobKind, TRACEL_DESCRIBE};
pub use report::{ReportedExperiment, RunReport, RunStatus, TRACEL_REPORT_FILE};

/// The protocol version a [`DefinitionsFile`] and a [`RunReport`] follow, which each gives as
/// `protocol`.
pub const PROTOCOL: u32 = 1;
