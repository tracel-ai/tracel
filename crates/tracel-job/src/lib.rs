#![deny(missing_docs)]

//! The process interface of a program's jobs, which the program that launches it shares with the
//! SDK: the files the program writes for its launcher, the environment variables that name them,
//! and its command line, with the plain data types they carry, [`JobDefinition`] and
//! [`ReportedExperiment`].
//!
//! - [`DefinitionsFile`]: the jobs a program can run, each given by its [`JobDefinition`]. When
//!   [`TRACEL_DESCRIBE`] names a path, the program writes the file there instead of running a
//!   job.
//! - [`RunReport`]: how a job is going and how it ended, and the experiments it started. When
//!   [`TRACEL_REPORT_FILE`] names a path, the runner of the job writes its report there when the
//!   job starts, again each time the job starts an experiment, and again when the job ends. When
//!   [`TRACEL_JOB_NUM`] gives the job a number, each experiment it starts records it as its
//!   [`JOB_NUM_ATTRIBUTE`], which links the experiment back to the job.
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
//! A run report:
//!
//! ```json
//! {
//!   "protocol": 1,
//!   "job": "train",
//!   "status": "completed",
//!   "started_at": "2026-10-06T14:02:11Z",
//!   "finished_at": "2026-10-06T14:31:40Z",
//!   "error": null,
//!   "experiments": [
//!     { "num": 42, "url": "https://console.tracel.ai/users/me/projects/demo/experiments/42" }
//!   ]
//! }
//! ```
//!
//! `experiments` is `[]` until the job starts an experiment, and stays `[]` for a job that starts
//! none.

mod command;
mod definitions;
mod flags;
mod json_file;
mod report;

pub use command::{command, completions, job_command, job_input};
pub use definitions::{DefinitionsFile, JobDefinition, TRACEL_DESCRIBE};
pub use report::{
    JOB_NUM_ATTRIBUTE, ReportedExperiment, RunReport, RunStatus, TRACEL_JOB_NUM, TRACEL_REPORT_FILE,
};

/// The protocol version a [`DefinitionsFile`] and a [`RunReport`] follow, which each gives as
/// `protocol`.
pub const PROTOCOL: u32 = 1;
