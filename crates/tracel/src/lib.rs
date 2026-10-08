// #![cfg_attr(not(feature = "std"), no_std)]
#![deny(missing_docs)]

//! # Tracel SDK
//!
//! High-level Tracel SDK.
//!
//! The SDK gives a program clients for the platform's services, which it picks from the
//! environment, and the job protocol through which the program's jobs are run, launched remotely
//! or run locally.
//!
//! Features:
//! - Artifact creation and management
//! - Experiment tracking
//! - Logging metrics
//! - Model versioning
//!
//! ## Crate Layout
//!
//! [`Target`] says where a program records its experiments and reaches models, datasets and
//! inference telemetry: offline under `./runs` by default, or a console project.
//! [`Target::from_env`] reads it from `TRACEL_TARGET` and the variables that target needs.
//!
//! The most commonly used re-exports are:
//! - [`experiment`]: experiment runs, logging, artifacts, and Burn learner integrations.
//! - [`app`]: jobs, plain functions of their input, and the entry point that runs one.
//! - [`job`]: the job definitions file, the run report, and the command line built from job
//!   definitions, which programs that launch jobs share with the SDK.
//! - [`console`]: organizations, projects, and users on the Tracel console.
//! - [`datasets`]: the dataset domain and its registry.
//! - [`models`]: the model domain and its registry.
//! - [`artifact`]: bundle and artifact utilities.
//!
//! ## Jobs
//!
//! A job is a function of its typed input. Register a program's jobs by name with
//! [`app::Jobs`], and return its [`run`](app::Jobs::run) from `main`: the program runs the job its
//! command line names. A job starts experiments, or none, with the clients a [`Target`] builds:
//!
//! ```no_run
//! use std::process::ExitCode;
//!
//! use serde::{Deserialize, Serialize};
//! use tracel::Target;
//! use tracel::app::{BoxError, Job, Jobs};
//!
//! #[derive(Default, Serialize, Deserialize)]
//! struct TrainingConfig {
//!     epochs: usize,
//! }
//!
//! fn train(config: TrainingConfig) -> Result<(), BoxError> {
//!     let experiments = Target::from_env()?.experiments()?;
//!     experiments.run("train", |run| {
//!         run.log_args(&config)?;
//!         // Your training code here
//!         Ok(())
//!     })
//! }
//!
//! fn main() -> ExitCode {
//!     Jobs::new()
//!         .version(env!("CARGO_PKG_VERSION"))
//!         .job(
//!             "train",
//!             Job::new(train)
//!                 .with_description("Train the model")
//!                 .with_default(TrainingConfig::default()),
//!         )
//!         .run()
//! }
//! ```
//!
//! Run it as `<binary> train '{"epochs": 5}'`: the job's name selects it, and its input, one JSON
//! document, is merged onto its default. Left out, the job runs with the default. A function
//! registered without [`Job`](app::Job) is a job with no description or default.
//!
//! [`Experiments::run`](experiment::Experiments::run) ends the experiment by what its code
//! returns: completed on `Ok`, and failed on `Err` or a panic.
//! [`Experiments::start`](experiment::Experiments::start) returns the run instead, for code that
//! ends it with `finish()` or `fail()`; a run dropped before either ends as failed.
//! [`log_args`](experiment::ExperimentRun::log_args) records the job's input as the experiment's
//! arguments, which the console lists as the experiment's config.
//!
//! The experiments [`Target::experiments`] builds while a job runs are linked to the job: each one
//! started from them records the job's number, is asked to stop when the job is, and is listed in
//! the job's run report, as [Launching Jobs](#launching-jobs) describes. Experiments built another
//! way, such as [`console::ProjectHandle::experiments`], are not linked unless the code links
//! them, through [`Experiments::attribute`](experiment::Experiments::attribute),
//! [`cancel_token`](experiment::Experiments::cancel_token) and
//! [`on_start`](experiment::Experiments::on_start).
//!
//! [`version`](app::Jobs::version) gives the binary its `--version`, which prints the binary's
//! name and that version; without it, the binary has no `--version`.
//!
//! Without a [`Target`], build the services from an adapter directly:
//! [`console::ProjectHandle::from_env`] for a console project, or
//! [`experiment::local::LocalExperiments`] to record offline.
//!
//! ## Command-Line Flags
//!
//! [`app::Jobs`] builds its command line from the definitions of its jobs, so each field of a
//! job's input is also a flag: `<binary> train --epochs 5` runs the job above with the same input
//! as `<binary> train '{"epochs": 5}'`. `<binary> --help` lists the jobs, and
//! `<binary> train --help` the job's flags with their defaults:
//!
//! ```text
//! Train the model
//!
//! Usage: my-binary train [OPTIONS] [INPUT]
//!
//! Arguments:
//!   [INPUT]  JSON merged after --config and before the flags
//!
//! Options:
//!       --epochs <INT>   [default: 0]
//!   -c, --config <FILE>  JSON merged before the input and the flags
//!   -h, --help           Print help
//! ```
//!
//! - With an input schema (`Job::with_schema` and the `schema` feature), a flag takes its field's
//!   type, allowed values and requiredness from the schema, and the field's doc comment as its
//!   help. Without one, each field of the default input is a flag typed by its value: a boolean,
//!   an integer, a number or a string. Burn `Config` types work this way.
//! - A nested field's flag joins the keys with dots and writes `_` as `-`: the field
//!   `optimizer.weight_decay` is `--optimizer.weight-decay`. A field that is an array, an object
//!   with no fields of its own, or `null` in the default input takes a JSON literal, such as
//!   `--layers '[64, 32]'`. A boolean flag given no value is `true`.
//! - The input is the default input, then the `--config` file, then the JSON document, then the
//!   flags, each merged onto the one before, so a later one wins. Launchers pass the JSON document.
//! - `help`, `version` and `config` are the command line's names: a field with one of them has no
//!   flag, and is set through `--config` or the JSON document.
//! - `<binary> --completions <SHELL>` prints the completion script for `bash`, `elvish`, `fish`,
//!   `powershell` or `zsh`.
//!
//! [`job::command`] builds the same command line from job definitions alone, such as those of a
//! definitions file, and [`job::job_input`] reads a job's input from its arguments.
//!
//! ## Describing Jobs
//!
//! With `TRACEL_DESCRIBE=<path>` set, the program writes the definitions of its jobs to `<path>`
//! as JSON, then exits without running a job: see [`job::DefinitionsFile`]. Each definition gives
//! the job's name, description, input schema and example input. The example input is the job's
//! default, and the input schema needs `Job::with_schema` and the `schema` feature.
//!
//! ## Launching Jobs
//!
//! A program whose `main` returns [`Jobs::run`](app::Jobs::run) can be launched by another
//! program, such as the `tracel` CLI, which reads its exit code:
//!
//! | Exit code | Meaning |
//! | --- | --- |
//! | 0 | The job completed, the help, version or completion script was printed, or the definitions file was written |
//! | 1 | The job failed |
//! | 2 | No job or an unknown job is named, and stderr lists the job names; or a flag or the input is unusable or does not decode |
//! | 130 | The job was asked to stop |
//!
//! With `TRACEL_REPORT_FILE=<path>` set, the program writes the job's [`RunReport`](job::RunReport)
//! to `<path>`: how the job is going and how it ended, and the experiments it started. It is
//! written when the job starts, again each time the job starts an experiment, and again when the
//! job ends, each time to `<path>.tmp` first, renamed to `<path>`. Without the variable, or when
//! the program rejects the job or its input, nothing is written:
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
//! `status` is `running`, `completed` or `failed`, by what the job returned, and `error` says why
//! a job failed. `experiments` lists the experiments the job started from the experiments a
//! [`Target`] built, in order: `[]` until it starts one, and for a job that starts none. `url` is
//! an experiment's page on the console; offline it is `null`, and `dir` gives the experiment's
//! directory instead, where `status.json` says how the experiment went and `events.jsonl` holds
//! its events, one JSON object per line, as
//! [`LocalExperiments`](experiment::local::LocalExperiments) describes.
//!
//! With `TRACEL_JOB_NUM` set, each of these experiments records it as its `tracel.job_num`
//! attribute, which links it to the job.
//!
//! SIGTERM, SIGINT or SIGHUP, or Ctrl-C on Windows, asks the running job to stop:
//! [`app::stop_requested`] turns `true`, and what [`app::on_stop`] registered runs. Through it,
//! each of these experiments has its cancel token cancelled, which stops a Burn learner given its
//! `interrupter()`, and logs a warning that a stop was requested; none starts after. The job ends
//! as `completed` or `failed` by what it returns, and the program exits with code 130. A second
//! signal ends the program at once. Launchers send SIGKILL after a 30-second grace period.

mod running_job;
mod target;

pub use target::{Target, TargetError};

/// Experiment tracking and management.
#[doc(inline)]
pub use tracel_experiment as experiment;

/// The Tracel console: its organizations, projects, and users.
#[doc(inline)]
pub use tracel_console as console;

/// A Tracel Station: experiments, models, and datasets served by one Station URL (requires the
/// `station` feature).
#[cfg(feature = "station")]
#[doc(inline)]
pub use tracel_station as station;

/// The dataset domain the console or a Station serves, and its registry.
#[doc(inline)]
pub use tracel_datasets as datasets;

/// The model domain the console or a Station serves, and its registry.
#[doc(inline)]
pub use tracel_models as models;

/// Inference contracts and adapters.
#[doc(hidden)]
#[doc(inline)]
pub use tracel_inference as inference;

/// Artifact bundle utilities and adapters.
#[doc(inline)]
pub use tracel_artifact as artifact;

/// Jobs, plain functions of their input, and the entry point that runs one.
#[doc(inline)]
pub use tracel_app as app;

/// The job definitions file, the run report, and the command line built from job definitions.
#[doc(inline)]
pub use tracel_job as job;
