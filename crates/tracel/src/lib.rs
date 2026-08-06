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
//! The most commonly used re-exports are:
//! - `experiment`: experiment runs, logging, artifacts, and Burn learner integrations.
//! - `app`: job registration, plus CLI and HTTP server front-ends to run those jobs.
//! - `artifact`: bundle and artifact utilities.
//! - `client`: HTTP client for the Tracel console.
//!
//! ## Features
//!
//! The default `sdk` feature covers everything above. Applications that only talk to the
//! console can depend on `client` alone (`default-features = false`), which leaves out
//! `experiment`, `app` and Burn with them.
//!
//! ## Registering Routines
//!
//! Wrap a routine into a job with `experiment::ExperimentModule::create`, then register it
//! with a `app::cli::Cli` (to dispatch from the command line) or a `app::server::Server`
//! (to dispatch over HTTP, requires the `server` feature):
//!
//! ```ignore
//! use tracel::app::cli::Cli;
//! use tracel::app::cli::mapper::JsonMapper;
//! use tracel::experiment::ExperimentRun;
//! use tracel::{Connection, Context};
//!
//! fn main() -> anyhow::Result<()> {
//!     let module = Context::new(Connection::Cloud)?.experiment();
//!
//!     let job = module.create("my_training_procedure", |session: &ExperimentRun, config| {
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

#[cfg(feature = "client")]
pub use tracel_client::Env;
#[cfg(feature = "client")]
pub use tracel_client::TracelCredentials;

/// HTTP client for the Tracel console.
#[cfg(feature = "client")]
#[doc(inline)]
pub use tracel_client as client;

/// Experiment tracking and management.
#[cfg(feature = "sdk")]
pub mod experiment {
    pub use tracel_core::experiment::*;
    #[doc(inline)]
    pub use tracel_experiment::*;
}

/// Dataset streaming (Station-only).
#[cfg(feature = "sdk")]
pub mod dataset {
    pub use tracel_core::{
        DatasetError, DatasetItem, DatasetModule, DatasetVersionSpec, DownloadedDataset,
        StreamedDataset,
    };
}

/// Inference contracts and adapters.
#[cfg(feature = "sdk")]
#[doc(hidden)]
#[doc(inline)]
pub use tracel_inference as inference;

/// On-device fleet synchronization helpers.
#[cfg(feature = "sdk")]
#[doc(hidden)]
#[doc(inline)]
pub use tracel_fleet as fleet;

/// Artifact bundle utilities and adapters.
#[cfg(feature = "client")]
#[doc(inline)]
pub use tracel_artifact as artifact;

/// App module for job registration, CLI, and config mappers
#[cfg(feature = "sdk")]
#[doc(inline)]
pub use tracel_app as app;

/// Station runner front-end: serve registered jobs to a Tracel Station job queue (requires the
/// `runner` feature).
#[cfg(feature = "runner")]
#[doc(inline)]
pub use tracel_runner as runner;

#[cfg(feature = "sdk")]
pub use tracel_core::Connection;
#[cfg(feature = "sdk")]
pub use tracel_core::Context;
#[cfg(feature = "sdk")]
pub use tracel_core::ContextError;
