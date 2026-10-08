//! The runner for Tracel jobs: run registered jobs from a command line.
//!
//! A [`Job`] is what the runner runs: it takes JSON input and ends as completed or failed.
//! Recording an experiment is what a job does as it runs. A job is registered from an
//! `ExperimentJob` and a [`Mapper`](mapper::Mapper) that decodes its input, which the runner hands
//! over as JSON. Each registered job has a [`JobDefinition`](tracel_job::JobDefinition): its name,
//! description and input.
//!
//! [`cli::Cli`] runs one job from the command line: `<job_name> [<input-json>] [<flags>]`, with a
//! flag per field of the job's input, and returns the process's exit code: 0 when the job
//! completed, 1 when it failed, 2 for an unknown job or an unusable flag or input, and 130 when it
//! was asked to stop. [`tracel_job::command`] builds that command line from the job definitions
//! alone, so a program that reads a [`DefinitionsFile`](tracel_job::DefinitionsFile) builds the
//! same one.
//!
//! An experiment records the input its mapper resolved, such as the input merged onto the
//! mapper's default, as its arguments.
//!
//! ## Reporting on a job
//!
//! When `TRACEL_REPORT_FILE` names a path, [`cli::Cli`] writes the
//! [`RunReport`](tracel_job::RunReport) of the job it runs there: how the job is going and how it
//! ended, and the experiments it recorded. It writes the report when the job starts, once its input
//! is decoded, again each time the job records an experiment, which the job hands to the reporter
//! of its [`JobContext`], and again when the job ends, as `completed` or `failed` by what the job
//! returned, whether or not it was asked to stop. The report of a job that records no experiment
//! gives its `experiments` as `[]`. Each write goes to `<path>.tmp` first, renamed to `<path>`.
//!
//! ## Describing jobs
//!
//! When `TRACEL_DESCRIBE` names a path, the runner writes a
//! [`DefinitionsFile`](tracel_job::DefinitionsFile) there, as JSON, instead of running a job, and
//! returns successfully. It writes `<path>.tmp` first and renames it to `<path>`, so the file is
//! never seen half written. While `TRACEL_DESCRIBE` is set, `ExperimentJob::run` returns an error
//! without creating an experiment.
//!
//! Each job's `input_example` comes from
//! [`JsonMapper::with_default`](mapper::JsonMapper::with_default), and its `input_schema` from
//! `JsonMapper::with_schema` (requires the `schema` feature).

mod adapter;
/// Command-line runner.
pub mod cli;
mod describe;
mod job;
pub mod mapper;
mod panics;
mod registry;
#[cfg(feature = "station")]
pub mod station;
#[cfg(test)]
mod test_support;

pub use describe::DescribeError;
pub use job::{BoxError, ExperimentReporter, IntoJob, Job, JobContext, PreparedJob};
pub use registry::JobRegistry;
