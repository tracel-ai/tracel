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
//! - [`app`]: job registration, plus CLI and HTTP server front-ends to run those jobs.
//! - [`console`]: organizations, projects, and users on the Tracel console.
//! - [`datasets`]: the dataset domain and its registry.
//! - [`models`]: the model domain and its registry.
//! - [`artifact`]: bundle and artifact utilities.
//!
//! ## Registering Routines
//!
//! Wrap a routine into a job with [`experiment::Experiments::create`], then register it
//! with a [`app::cli::Cli`] (to dispatch from the command line) or a `app::server::Server`
//! (to dispatch over HTTP, requires the `server` feature):
//!
//! ```ignore
//! use tracel::Target;
//! use tracel::app::cli::Cli;
//! use tracel::app::cli::mapper::JsonMapper;
//! use tracel::experiment::ExperimentRun;
//!
//! fn main() -> anyhow::Result<()> {
//!     let experiments = Target::from_env()?.experiments()?;
//!
//!     let job = experiments.create("my_training_procedure", |session: &ExperimentRun, config| {
//!         // Your training code here
//!         my_training_function(session, config)
//!     });
//!
//!     Cli::new()
//!         .register(job, JsonMapper::with_default(MyConfig::default()))
//!         .run()?;
//!
//!     Ok(())
//! }
//! ```
//!
//! The job's name (`"my_training_procedure"` above) is what callers use to select it, whether
//! from the CLI or from an HTTP request path.
//!
//! Without a [`Target`], build the services from an adapter directly:
//! [`console::ProjectHandle::from_env`] for a console project, or
//! [`experiment::local::LocalExperiments`] to record offline.

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

/// App module for job registration, CLI, and config mappers
#[doc(inline)]
pub use tracel_app as app;

/// Station runner front-end: serve registered jobs to a Tracel Station job queue (requires the
/// `runner` feature).
#[cfg(feature = "runner")]
#[doc(inline)]
pub use tracel_runner as runner;
