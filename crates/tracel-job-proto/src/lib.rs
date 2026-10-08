#![deny(missing_docs)]

//! The contract between the process adapter of the Tracel runtime and the programs that launch job
//! binaries, such as `tracel run` and compute providers: the command line of a job binary, the
//! environment variables a launcher sets, the files the binary writes for it and its exit code,
//! with the plain data types they carry, [`JobDefinition`] and [`ReportedExperiment`]. It is not
//! an API for writing jobs.
//!
//! It holds the contract's shapes and names, and does no I/O: [`DefinitionsFile::to_json`] and
//! [`RunReport::to_json`] give the exact text of each file, [`DefinitionsFile::from_json`] and
//! [`RunReport::from_json`] read it back, checking its [`PROTOCOL`], and [`job_input`] reads the
//! `--config` file through the function its caller gives. Writing and reading the files is the
//! caller's.
//!
//! ## Running a job
//!
//! `<binary> <job> [<input-json>] [<flags>]` runs the job `<job>`. Its input is the job's default
//! input, then the `--config` file, then the JSON document, then the flags, each merged onto the
//! one before. A launcher passes the input as the JSON document. [`command`] builds the binary's
//! command line from its jobs' definitions alone, so a launcher that reads a definitions file
//! parses the same flags as the binary that wrote it, and [`job_input`] reads a job's input from
//! the arguments it parsed.
//!
//! The binary's exit code says how the job ended:
//!
//! | Exit code | Meaning |
//! | --- | --- |
//! | 0 | The job completed, the help, version or completion script was printed, or the definitions file was written |
//! | 1 | The job failed, or the definitions file or the run report could not be written |
//! | 2 | No job or an unknown job is named, and stderr lists the job names; or a flag or the input is unusable or does not decode |
//! | 130 | The job was asked to stop |
//!
//! SIGTERM, SIGINT or SIGHUP, or Ctrl-C on Windows, asks the running job to stop. The job ends as
//! completed or failed by what it returns, and the binary exits with code 130. A second signal
//! ends the binary at once. A launcher sends SIGKILL after a grace period; `tracel run` waits 30
//! seconds.
//!
//! ## The launch environment
//!
//! - [`TRACEL_DESCRIBE`]: when it names a path, the binary writes its [`DefinitionsFile`] there
//!   instead of running a job, and exits with code 0.
//! - [`TRACEL_REPORT_FILE`]: when it names a path, the binary writes the job's [`RunReport`] there
//!   when the job starts, once its input decodes, again each time the job starts an experiment,
//!   and again when the job ends. A command line or an input the binary rejects writes none.
//! - [`TRACEL_JOB_NUM`]: the number the launcher gave the job. Each experiment the job starts
//!   records it as its [`JOB_NUM_ATTRIBUTE`], `tracel.job_num`, a number when it is one, which
//!   links the experiment back to the job.
//!
//! Both files are JSON and give the [`PROTOCOL`] version they follow. The binary writes each to
//! `<path>.tmp` first, then renames it to `<path>`, so it is never read half written.
//!
//! ## The definitions file
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
//! ## The run report
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
//! `status` is `running` until the job ends, then `completed` or `failed` by what the job
//! returned, whether or not it was asked to stop, and `error` says why a job failed.
//! `experiments` lists the experiments the job started, in order: `[]` until it starts one, and
//! for a job that starts none. `url` is an experiment's page on the console; offline it is `null`,
//! and `dir` gives the experiment's directory instead, where `status.json` says how the experiment
//! went and `events.jsonl` holds its events, one JSON object per line.

mod command;
mod definitions;
mod flags;
mod json;
mod report;

pub use command::{ConfigError, command, completions, job_command, job_input};
pub use definitions::{DefinitionsFile, JobDefinition, TRACEL_DESCRIBE};
pub use json::ParseError;
pub use report::{
    JOB_NUM_ATTRIBUTE, ReportedExperiment, RunReport, RunStatus, TRACEL_JOB_NUM, TRACEL_REPORT_FILE,
};

/// The protocol version a [`DefinitionsFile`] and a [`RunReport`] follow, which each gives as
/// `protocol`.
pub const PROTOCOL: u32 = 1;
