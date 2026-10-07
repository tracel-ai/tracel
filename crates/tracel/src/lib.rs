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
//! [`Target::from_env`] reads it from `TRACEL_CONNECTION` and the variables that target needs.
//!
//! The most commonly used re-exports are:
//! - [`experiment`]: experiment runs, logging, artifacts, and Burn learner integrations.
//! - [`app`]: job registration, plus the command-line and HTTP runners that run those jobs.
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
//!     experiment.log_args(&config)?;
//!     // Your training code here
//!     Ok(())
//! }
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let train = Target::from_env()?
//!         .experiments()?
//!         .create("train", train)
//!         .with_description("Train the model");
//!
//!     Cli::new()
//!         .register(train, JsonMapper::with_default(TrainingConfig::default()))
//!         .run()?;
//!     Ok(())
//! }
//! ```
//!
//! Run it as `<binary> train '{"epochs": 5}'`: the job's name selects it, and its input, one JSON
//! document, is merged onto the default. Left out, the job runs with the default. Over HTTP, the
//! same input is the body of `POST /train`.
//!
//! Without a [`Target`], build the services from an adapter directly:
//! [`console::ProjectHandle::from_env`] for a console project, or
//! [`experiment::local::LocalExperiments`] to record offline.
//!
//! ## Describing Jobs
//!
//! With `TRACEL_DESCRIBE=<path>` set, every runner writes the definitions of its jobs to `<path>`
//! as JSON, then returns without running a job or serving: see [`app::DefinitionsFile`]. Each
//! definition gives the job's name, kind (`experiment` or `inference`), description, input schema
//! and example input. The example input is the mapper's default, and the input schema needs
//! `JsonMapper::with_schema` and the `schema` feature. While the variable is set,
//! `ExperimentJob::run` returns an
//! [`ExperimentErrorKind::Describing`](experiment::error::ExperimentErrorKind::Describing) error
//! without creating an experiment, so describing a program never trains.

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

/// Station runner: serve registered jobs to a Tracel Station job queue (requires the `runner`
/// feature).
#[cfg(feature = "runner")]
#[doc(inline)]
pub use tracel_runner as runner;
