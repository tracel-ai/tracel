//! A program's jobs, by name.

use std::collections::BTreeMap;

use serde::de::DeserializeOwned;
use serde_json::Value;
use tracel_job_proto::JobDefinition;

use crate::execution::{self, PreparedJob};
use crate::job::{IntoJob, Prepared, Registered};

/// A program's jobs, by name, and how the program runs the one its command line names: `main`
/// returns [`run`](Self::run).
///
/// The command line is `<program> <job> [<input-json>] [<flags>]`, built from the jobs'
/// definitions: one subcommand per job, taking its input as a JSON document or a `--config` file.
/// A job given its input's schema, with `Job::with_schema` (requires the `schema` feature), also
/// has a flag per field of its input, typed by the schema, so it runs without writing JSON.
/// `<program> <job> --help` lists the flags. For a job whose input has the fields `epochs` and
/// `optimizer.weight_decay`:
///
/// ```text
/// <program> train --epochs 5 --optimizer.weight-decay 0.01
/// <program> train '{"epochs": 5}'
/// <program> train -c config.json --epochs 5
/// ```
///
/// The input is the `--config` file, the JSON document and the flags, each merged onto the one
/// before, then merged onto the job's default input. With none, the job runs with its default
/// input, or with `null` when it has none, which a job taking `()` accepts.
///
/// `<program> --completions <SHELL>` prints the program's completion script for `bash`,
/// `elvish`, `fish`, `powershell` or `zsh`, and with [`version`](Self::version),
/// `<program> --version` prints the program's name and version.
///
/// ```no_run
/// use std::process::ExitCode;
///
/// use serde::{Deserialize, Serialize};
/// use tracel_runtime::{BoxError, Job, Jobs};
///
/// #[derive(Serialize, Deserialize)]
/// struct TrainingConfig {
///     epochs: u32,
/// }
///
/// fn train(config: TrainingConfig) -> Result<(), BoxError> {
///     for epoch in 1..=config.epochs {
///         if tracel_runtime::stop_requested() {
///             break;
///         }
///         println!("epoch {epoch}");
///     }
///     Ok(())
/// }
///
/// fn evaluate(_: ()) -> Result<(), BoxError> {
///     Ok(())
/// }
///
/// fn main() -> ExitCode {
///     Jobs::new()
///         .version(env!("CARGO_PKG_VERSION"))
///         .job(
///             "train",
///             Job::new(train)
///                 .with_description("Train the model")
///                 .with_default(TrainingConfig { epochs: 10 }),
///         )
///         .job("evaluate", evaluate)
///         .run()
/// }
/// ```
#[derive(Default)]
pub struct Jobs {
    jobs: BTreeMap<String, Registered>,
    default: Option<String>,
    version: Option<String>,
}

impl Jobs {
    /// No jobs.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `job` as `name`: a [`Job`](crate::Job), or a function of the job's input, which
    /// is the job with no description, default input or input schema.
    ///
    /// # Panics
    ///
    /// When a job named `name` is already registered.
    pub fn job<I>(mut self, name: impl Into<String>, job: impl IntoJob<I>) -> Self
    where
        I: DeserializeOwned + Send + 'static,
    {
        let name = name.into();
        if self.jobs.contains_key(&name) {
            panic!("job '{name}' is already registered");
        }
        let job = Registered::new(name.clone(), job.into_job());
        self.jobs.insert(name, job);
        self
    }

    /// Names the registered job to run, with no input, when the command line names none.
    pub fn default_job(mut self, name: impl Into<String>) -> Self {
        self.default = Some(name.into());
        self
    }

    /// Gives the program's version, such as `env!("CARGO_PKG_VERSION")`, which
    /// `<program> --version` prints after the program's name. Without one, the command line
    /// has no `--version`.
    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    /// The definitions of the registered jobs, ordered by name.
    pub fn definitions(&self) -> impl Iterator<Item = &JobDefinition> {
        self.jobs.values().map(Registered::definition)
    }

    /// The program's version, when [`version`](Self::version) gave one.
    pub fn get_version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    /// The job to run when the command line names none, when
    /// [`default_job`](Self::default_job) named one.
    pub fn get_default_job(&self) -> Option<&str> {
        self.default.as_deref()
    }

    /// The names of the registered jobs, in order.
    fn names(&self) -> Vec<String> {
        self.jobs.keys().cloned().collect()
    }

    /// The job `name`, ready to run with `input` merged onto its default input; `Value::Null`
    /// stands for no input.
    ///
    /// This is how a transport runs a job: prepare it with the name and input its launcher gave,
    /// then [`run`](PreparedJob::run) it, launched as an [`Execution`](crate::Execution) says.
    ///
    /// # Errors
    ///
    /// [`PrepareError::UnknownJob`] when no job has that name, and
    /// [`PrepareError::InvalidInput`] when the input does not decode as the job's input. Nothing
    /// runs either way.
    pub fn prepare(&self, name: &str, input: Value) -> Result<PreparedJob, PrepareError> {
        let job = self
            .jobs
            .get(name)
            .ok_or_else(|| PrepareError::UnknownJob {
                name: name.to_string(),
                available: self.names(),
            })?;
        let Prepared { input, run } = job.prepare(input).map_err(PrepareError::InvalidInput)?;
        Ok(execution::prepared(name.to_string(), input, run))
    }
}

/// Why [`Jobs::prepare`] gave no job to run.
#[derive(Debug, thiserror::Error)]
pub enum PrepareError {
    /// No registered job has the name given.
    #[error("unknown job '{name}'")]
    UnknownJob {
        /// The name given.
        name: String,
        /// The names of the registered jobs, in order.
        available: Vec<String>,
    },

    /// The input does not decode as the job's input.
    #[error("invalid input: {0}")]
    InvalidInput(#[source] serde_json::Error),
}
