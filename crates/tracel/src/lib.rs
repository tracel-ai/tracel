// #![cfg_attr(not(feature = "std"), no_std)]
#![deny(missing_docs)]

//! # Tracel SDK
//!
//! High-level Tracel SDK.
//!
//! The SDK gives a program clients for the platform's services, which it picks from the
//! environment, and the runtime that runs the program's jobs, launched remotely or run locally.
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
//! - [`runtime`]: jobs, plain functions of their input, and the entry point that runs one.
//! - [`console`]: organizations, projects, and users on the Tracel console.
//! - [`datasets`]: the dataset domain and its registry.
//! - [`models`]: the model domain and its registry.
//! - [`artifact`]: bundle and artifact utilities.
//!
//! ## Jobs
//!
//! A job is a function of its typed input. Register a program's jobs by name with
//! [`runtime::Jobs`], and return its [`run`](runtime::Jobs::run) from `main`: the program runs the
//! job its command line names. A job starts experiments, or none, with the clients a [`Target`]
//! builds:
//!
//! ```no_run
//! use std::process::ExitCode;
//!
//! use serde::{Deserialize, Serialize};
//! use tracel::Target;
//! use tracel::runtime::{BoxError, Job, Jobs};
//!
//! #[derive(Default, Serialize, Deserialize)]
//! struct TrainingConfig {
//!     epochs: usize,
//! }
//!
//! fn train(config: TrainingConfig) -> Result<(), BoxError> {
//!     let experiments = Target::from_env()?.experiments()?;
//!     experiments.run("train", |run| {
//!         // Your training code here
//!         run.log_info(format!("training for {} epochs", config.epochs));
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
//! registered without [`Job`](runtime::Job) is a job with no description or default.
//!
//! [`Experiments::run`](experiment::Experiments::run) ends the experiment by what its code
//! returns: completed on `Ok`, and failed on `Err` or a panic.
//! [`Experiments::start`](experiment::Experiments::start) returns the run instead, for code that
//! ends it with `finish()` or `fail()`; a run dropped before either ends as failed.
//! An experiment started in a job, from the experiments a [`Target`] built, records the job's
//! input, merged onto its default, as its arguments, which the console lists as the experiment's
//! config; [`log_args`](experiment::ExperimentRun::log_args) replaces them.
//!
//! The experiments [`Target::experiments`] builds in a job, on the thread that runs it, are linked
//! to that job, even for runs started on other threads: each one started from them records the
//! job's number and input. Experiments built another way, such as
//! [`console::ProjectHandle::experiments`], are not linked unless the code links them, through
//! [`Experiments::attribute`](experiment::Experiments::attribute) and
//! [`arguments`](experiment::Experiments::arguments).
//!
//! [`version`](runtime::Jobs::version) gives the binary its `--version`, which prints the binary's
//! name and that version; without it, the binary has no `--version`.
//!
//! Without a [`Target`], build the services from an adapter directly:
//! [`console::ProjectHandle::from_env`] for a console project, or
//! [`experiment::local::LocalExperiments`] to record offline.
//!
//! ## Command-Line Flags
//!
//! A job given its input's JSON Schema has a flag per field of its input. With
//! `schemars::JsonSchema` derived for its input type, the `schema` feature enabled and
//! `.with_schema()` added to the job above, `<binary> train --epochs 5` runs it with the same
//! input as `<binary> train '{"epochs": 5}'`. `<binary> --help` lists the jobs, and
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
//! - A flag takes its field's type, allowed values and requiredness from the schema, and the
//!   field's doc comment as its help. A job without a schema has no flags: its input is the JSON
//!   document or the `--config` file.
//! - A nested field's flag joins the keys with dots and writes `_` as `-`: the field
//!   `optimizer.weight_decay` is `--optimizer.weight-decay`. A field that is an array, a map, an
//!   optional object or an enum with data takes a JSON literal, such as `--layers '[64, 32]'`. A
//!   boolean flag given no value is `true`, and takes a value only as `--flag=value`, such as
//!   `--shuffle=false`.
//! - The input is the default input, then the `--config` file, then the JSON document, then the
//!   flags, each merged onto the one before, so a later one wins. Launchers pass the JSON document.
//! - `help` and `config` are the command line's names: a field with one of them has no flag, and is
//!   set through `--config` or the JSON document.
//! - `<binary> --completions <SHELL>` prints the completion script for `bash`, `elvish`, `fish`,
//!   `powershell` or `zsh`.
//!
//! ## Launching Jobs
//!
//! `tracel run <job>` builds the program and runs one of its jobs, taking the job's flags on its
//! own command line, and `tracel run --list` lists the jobs with their descriptions. The program's
//! exit code says how the job ended:
//!
//! | Exit code | Meaning |
//! | --- | --- |
//! | 0 | The job completed, or the help, version or completion script was printed |
//! | 1 | The job failed |
//! | 2 | No job or an unknown job is named, and stderr lists the job names; or a flag or the input is unusable or does not decode |

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
pub use tracel_runtime as runtime;
