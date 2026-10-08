//! A program's jobs, and the adapters that run them.
//!
//! A job is a function of its typed input, `Fn(I) -> Result<(), E>`, registered by name in
//! [`Jobs`]. The core runs a job by name, with its input decoded from JSON, and knows nothing of
//! how it was asked to: an adapter does. A job is experiment-agnostic: it starts experiments, or
//! none, as its code does.
//!
//! - The process adapter: `main` returns [`Jobs::run`]. Run locally, or by `tracel run`, the
//!   program runs the one job its command line names, and exits with the code that says how the
//!   job ended.
//! - The Station adapter: `station::StationRunner` serves the jobs to a Tracel Station job queue
//!   (requires the `station` feature).
//!
//! [`Job`] adds a job's description, its default input, which its input is merged onto, and its
//! input's JSON Schema (requires the `schema` feature). The command line
//! `<program> <job> [<input-json>] [<flags>]` is built from them, with a flag per field of the
//! input, as [`Jobs`] describes.
//!
//! ## Running a job
//!
//! `<program> <job> [<input-json>]` runs the job with that input. It exits with code 0 when the
//! job completed, 1 when it failed, 2 when the job or its input is rejected, and 130 when it was
//! asked to stop. SIGTERM, SIGINT or SIGHUP, or Ctrl-C on Windows, asks the job to stop.
//!
//! ## The running job
//!
//! Each execution of a job has its own [`JobScope`]: the number or id its launcher gave it, its
//! input, and the request that it stop, which its adapter decides. The scope is current on the
//! thread that runs the job's function, while it runs, so jobs a Station runner runs at once each
//! see their own. The functions read it: [`job_running`] says whether there is one,
//! [`stop_requested`] whether the job was asked to stop, [`on_stop`] runs code once it is,
//! [`job_num`] gives its number, and [`job_input`] its input, as JSON. To read it on another
//! thread, hold [`JobScope::current`]. The `tracel` crate links the experiments a job starts to it
//! through its scope. While a thread runs no job, such as in a plain binary or a unit test, there
//! is none: [`job_running`] and [`stop_requested`] give `false`, [`job_num`] and [`job_input`]
//! give `None`, and [`on_stop`] does nothing.

mod job;
mod jobs;
mod panics;
mod scope;
mod stop;

pub use job::{BoxError, IntoJob, Job};
pub use jobs::Jobs;
#[cfg(feature = "station")]
pub use jobs::station;
pub use scope::{JobScope, job_input, job_num, job_running, on_stop, stop_requested};
