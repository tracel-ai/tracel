#![deny(missing_docs)]

//! A program's jobs, and the adapters that run them.
//!
//! A job is a function of its typed input, `Fn(I) -> Result<(), E>`, registered by name in
//! [`Jobs`]. The core runs a job by name, with its input decoded from JSON, and knows nothing of
//! how it was asked to: a transport does, through the one public API every transport uses, as
//! [Running jobs from your own transport](#running-jobs-from-your-own-transport) describes. A job
//! is experiment-agnostic: it starts experiments, or none, as its code does. Two adapters come
//! with the crate:
//!
//! - The process adapter: `main` returns [`Jobs::run`]. Run locally, or by `tracel run`, the
//!   program runs the one job its command line names, and exits with the code that says how the
//!   job ended.
//! - The Station adapter: `station::StationRunner` serves the jobs to a Tracel Station job queue
//!   (requires the `station` feature).
//!
//! [`Job`] adds a job's description, its default input, which its input is merged onto, and its
//! input's JSON Schema (requires the `schema` feature). The command line
//! `<program> <job> [<input-json>] [<flags>]` is built from them, with a flag per field of an
//! input that has a schema, as [`Jobs`] describes.
//!
//! ## Running a job
//!
//! `<program> <job> [<input-json>]` runs the job with that input. It exits with code 0 when the
//! job completed, 1 when it failed, and 2 when the job or its input is rejected.
//!
//! ## Running jobs from your own transport
//!
//! A transport of a program's own, such as its own server or a device agent, runs jobs through the
//! same API as the adapters above. [`Jobs::prepare`] gives the job a name names, with its input
//! merged onto its default and decoded, or the [`PrepareError`] that says why it cannot run: an
//! unknown job, with the registered names, or an input that does not decode, with the reason.
//! [`PreparedJob::run`] runs it on the current thread, launched as an [`Execution`] says: with the
//! number the transport gives it. The [`Outcome`] says whether it completed or failed, with its
//! error. [`Jobs`] can be shared between threads in an `Arc`, and a [`PreparedJob`] sent to the
//! thread that runs it.
//!
//! ```
//! use std::sync::Arc;
//!
//! use serde_json::json;
//! use tracel_runtime::{BoxError, Execution, JobStatus, Jobs};
//!
//! fn train(epochs: u32) -> Result<(), BoxError> {
//!     if epochs == 0 {
//!         return Err("no epochs to train".into());
//!     }
//!     Ok(())
//! }
//!
//! let jobs = Arc::new(Jobs::new().job("train", train));
//!
//! // The transport was asked to run job 42, `train`, with the input `3`.
//! let prepared = jobs.prepare("train", json!(3))?;
//! let execution = Execution::new().with_num("42");
//! let outcome = std::thread::spawn(move || prepared.run(execution))
//!     .join()
//!     .unwrap();
//! assert_eq!(outcome.status(), JobStatus::Completed);
//!
//! // A job fails by what it returns.
//! let failed = jobs.prepare("train", json!(0))?.run(Execution::new());
//! assert_eq!(failed.status(), JobStatus::Failed);
//! assert_eq!(failed.error().unwrap().to_string(), "no epochs to train");
//! # Ok::<(), tracel_runtime::PrepareError>(())
//! ```
//!
//! ## The running job
//!
//! Each execution of a job has its own [`JobScope`]: the number or id its transport gave it, as
//! its [`Execution`] says, and its input. The scope is current on the thread that runs the job's
//! function, while it runs, so jobs a Station runner runs at once each see their own. The
//! functions read it: [`job_running`] says whether there is one, [`job_num`] gives its number, and
//! [`job_input`] its input, as JSON. To read it on another thread, hold [`JobScope::current`]. The
//! `tracel` crate links the experiments a job starts to it through its scope. While a thread runs
//! no job, such as in a plain binary or a unit test, there is none: [`job_running`] gives `false`,
//! and [`job_num`] and [`job_input`] give `None`.

mod execution;
mod job;
mod jobs;
mod panics;
mod process;
mod scope;
#[cfg(feature = "station")]
pub mod station;

pub use execution::{Execution, JobStatus, Outcome, PreparedJob};
pub use job::{BoxError, IntoJob, Job};
pub use jobs::{Jobs, PrepareError};
pub use scope::{JobScope, job_input, job_num, job_running};
pub use tracel_job::JobDefinition;
