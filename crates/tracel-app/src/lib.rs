//! The SDK half of the Tracel job protocol: a program's jobs, and the adapters that run them.
//!
//! A job is a function of its typed input, `Fn(I) -> Result<(), E>`, registered by name in
//! [`Jobs`]. The core runs a job by name, with its input decoded from JSON, and knows nothing of
//! how it was asked to: an adapter does. A job is experiment-agnostic: it starts experiments, or
//! none, as its code does.
//!
//! - The process adapter: `main` returns [`Jobs::run`]. Run locally, or launched by a program
//!   such as the `tracel` CLI, the program runs the one job its command line names, and exits with
//!   the code that says how the job ended.
//! - The Station adapter: `station::StationRunner` serves the jobs to a Tracel Station job queue
//!   (requires the `station` feature).
//!
//! [`Job`] adds what a launcher reads about a job: its description, its default input, which its
//! input is merged onto and which is its example input, and its input's JSON Schema (requires the
//! `schema` feature). The command line `<program> <job> [<input-json>] [<flags>]` is built from
//! them, with a flag per field of the input, as [`tracel_job::command`] builds it, so a program
//! that reads a [`DefinitionsFile`](tracel_job::DefinitionsFile) builds the same one.
//!
//! ## The process interface
//!
//! - When `TRACEL_DESCRIBE` names a path, the program writes the
//!   [`DefinitionsFile`](tracel_job::DefinitionsFile) of its jobs there, as JSON, instead of
//!   running a job, and exits with code 0.
//! - `<program> <job> [<input-json>]` runs the job with that input. It exits with code 0 when the
//!   job completed, 1 when it failed, 2 when the job or its input is rejected, and 130 when it was
//!   asked to stop.
//! - When `TRACEL_REPORT_FILE` names a path, the program writes the job's
//!   [`RunReport`](tracel_job::RunReport) there when the job starts, again each time code in the
//!   job reports an experiment it started, and again when the job ends, as `completed` or `failed`
//!   by what the job returned, whether or not it was asked to stop. A job or input that is
//!   rejected writes none.
//! - SIGTERM, SIGINT or SIGHUP, or Ctrl-C on Windows, asks the job to stop. Launchers send SIGKILL
//!   after a 30-second grace period.
//!
//! ## The running job
//!
//! An adapter runs one job at a time, so while it runs a job, what the job is given is the
//! process's, for code on any thread: [`job_running`] says whether there is one,
//! [`stop_requested`] whether it was asked to stop, [`on_stop`] runs code once it is, [`job_num`]
//! gives the number `TRACEL_JOB_NUM` gives it, and [`report_experiment`] adds an experiment to its
//! run report. The `tracel` crate links the experiments a job starts to it through them. While no
//! job runs, such as in a plain binary or a unit test, there is none: [`job_running`] and
//! [`stop_requested`] give `false`, [`job_num`] gives `None`, and the rest do nothing.

mod job;
mod jobs;
mod panics;
mod running;
mod stop;
#[cfg(test)]
mod test_support;

pub use job::{BoxError, IntoJob, Job};
pub use jobs::Jobs;
#[cfg(feature = "station")]
pub use jobs::station;
pub use running::{job_num, job_running, on_stop, report_experiment, stop_requested};
