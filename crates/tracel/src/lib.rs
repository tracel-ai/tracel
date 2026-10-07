// #![cfg_attr(not(feature = "std"), no_std)]
#![deny(missing_docs)]

//! # Tracel SDK
//!
//! High-level Tracel SDK.
//!
//! This crate re-exports the main crates used to build training, experiment tracking, inference,
//! and fleet workflows on top of Tracel.
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
//! - [`app`]: job registration, plus the command-line and HTTP runners that run those jobs.
//! - [`job`]: the job definitions file, the run report, and the command line built from job
//!   definitions, which programs that launch jobs share with the SDK.
//! - [`console`]: organizations, projects, and users on the Tracel console.
//! - [`datasets`]: the dataset domain and its registry.
//! - [`models`]: the model domain and its registry.
//! - [`artifact`]: bundle and artifact utilities.
//!
//! ## Registering Jobs
//!
//! Wrap a routine into a job with [`experiment::Experiments::create`], then register it with a
//! runner: an [`app::cli::Cli`] runs it from the command line, and an `app::server::Server` serves
//! it over HTTP (requires the `server` feature). A mapper decodes the job's JSON input:
//!
//! ```no_run
//! use std::process::ExitCode;
//!
//! use serde::{Deserialize, Serialize};
//! use tracel::Target;
//! use tracel::app::cli::Cli;
//! use tracel::app::mapper::JsonMapper;
//! use tracel::experiment::ExperimentRun;
//!
//! #[derive(Default, Serialize, Deserialize)]
//! struct TrainingConfig {
//!     epochs: usize,
//! }
//!
//! fn train(
//!     experiment: &ExperimentRun,
//!     config: TrainingConfig,
//! ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//!     // Your training code here
//!     Ok(())
//! }
//!
//! fn main() -> Result<ExitCode, Box<dyn std::error::Error>> {
//!     let train = Target::from_env()?
//!         .experiments()?
//!         .create("train", train)
//!         .with_description("Train the model");
//!
//!     Ok(Cli::new()
//!         .version(env!("CARGO_PKG_VERSION"))
//!         .register(train, JsonMapper::with_default(TrainingConfig::default()))
//!         .run())
//! }
//! ```
//!
//! Run it as `<binary> train '{"epochs": 5}'`: the job's name selects it, and its input, one JSON
//! document, is merged onto the default. Left out, the job runs with the default. Over HTTP, the
//! same input is the body of `POST /train`. The experiment records the merged input as its
//! arguments, which the console lists as the experiment's config; `ExperimentJob::run` records
//! the input it is given.
//!
//! [`version`](app::cli::Cli::version) gives the binary its `--version`, which prints the
//! binary's name and that version; without it, the binary has no `--version`.
//!
//! Without a [`Target`], build the services from an adapter directly:
//! [`console::ProjectHandle::from_env`] for a console project, or
//! [`experiment::local::LocalExperiments`] to record offline.
//!
//! ## Command-Line Flags
//!
//! [`app::cli::Cli`] builds its command line from the definitions of its jobs, so each field of a
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
//! - With an input schema (`JsonMapper::with_schema` and the `schema` feature), a flag takes its
//!   field's type, allowed values and requiredness from the schema, and the field's doc comment
//!   as its help. Without one, each field of the example input is a flag typed by its value: a
//!   boolean, an integer, a number or a string. Burn `Config` types work this way.
//! - A nested field's flag joins the keys with dots and writes `_` as `-`: the field
//!   `optimizer.weight_decay` is `--optimizer.weight-decay`. A field that is an array, an object
//!   with no fields of its own, or `null` in the example input takes a JSON literal, such as
//!   `--layers '[64, 32]'`. A boolean flag given no value is `true`.
//! - The input is the example input, then the `--config` file, then the JSON document, then the
//!   flags, each merged onto the one before, so a later one wins. Launchers pass the JSON document.
//! - `help`, `version` and `config` are the runner's names: a field with one of them has no flag,
//!   and is set through `--config` or the JSON document.
//! - `<binary> --completions <SHELL>` prints the completion script for `bash`, `elvish`, `fish`,
//!   `powershell` or `zsh`.
//!
//! [`job::command`] builds the same command line from job definitions alone, such as those of a
//! definitions file, and [`job::job_input`] reads a job's input from its arguments. A job
//! whose input type derives `clap::Parser` can parse its own arguments instead, through
//! [`ClapMapper`](app::mapper::ClapMapper), from a JSON string: `<binary> train '"--epochs 3"'`.
//!
//! ## Describing Jobs
//!
//! With `TRACEL_DESCRIBE=<path>` set, every runner writes the definitions of its jobs to `<path>`
//! as JSON, then returns without running a job or serving: see [`job::DefinitionsFile`]. Each
//! definition gives the job's name, kind (`experiment` or `inference`), description, input schema
//! and example input. The example input is the mapper's default, and the input schema needs
//! `JsonMapper::with_schema` and the `schema` feature. While the variable is set,
//! `ExperimentJob::run` returns an
//! [`ExperimentErrorKind::Describing`](experiment::error::ExperimentErrorKind::Describing) error
//! without creating an experiment, so describing a program never trains.
//!
//! ## Launching Jobs
//!
//! A program that runs its jobs with [`app::cli::Cli`] can be launched by another program, such
//! as the `tracel` CLI. [`Cli::run`](app::cli::Cli::run) returns the exit code `main` returns:
//!
//! | Exit code | Meaning |
//! | --- | --- |
//! | 0 | The job completed, the help, version or completion script was printed, or the definitions file was written |
//! | 1 | The job failed |
//! | 2 | No job or an unknown job is named, and stderr lists the job names; or a flag or the input is unusable or does not decode |
//! | 130 | The job was asked to stop |
//!
//! With `TRACEL_REPORT_FILE=<path>` set, an experiment job writes a [`RunReport`](job::RunReport)
//! to `<path>` when it creates its experiment, and again when the run ends, each time to
//! `<path>.tmp` first, renamed to `<path>`. Without it, nothing is written:
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
//!
//! `status` is `running`, `completed` or `failed`, and `error` says why a run failed. `url` is the
//! experiment's page on the console; offline it is `null`, and `dir` gives the run's directory
//! instead, where `status.json` holds the same report and `events.jsonl` the run's events, one JSON
//! object per line, as [`LocalExperiments`](experiment::local::LocalExperiments) describes.
//!
//! With `TRACEL_JOB_NUM` set, the experiment records it as its `tracel.job_num` attribute, which
//! links it to the job that ran it.
//!
//! SIGTERM, SIGINT or SIGHUP, or Ctrl-C on Windows, asks the running job to stop: it cancels the
//! experiment's cancel token, which stops a Burn learner given its `interrupter()`, and the
//! experiment logs a warning that a stop was requested. The run ends as `completed` or `failed` by
//! what the job's function returns, and the program exits with code 130. A second signal ends the
//! program at once. Launchers send SIGKILL after a 30-second grace period.

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

/// Job registration, input mappers, and the command-line and HTTP runners.
#[doc(inline)]
pub use tracel_app as app;

/// The job definitions file, the run report, and the command line built from job definitions.
#[doc(inline)]
pub use tracel_job as job;

/// Station runner: serve registered jobs to a Tracel Station job queue (requires the `runner`
/// feature).
#[cfg(feature = "runner")]
#[doc(inline)]
pub use tracel_runner as runner;
