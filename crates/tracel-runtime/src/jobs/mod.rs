//! A program's jobs, and the adapters that run them: [`process`], the process the program runs as,
//! and [`station`], a Station runner (requires the `station` feature).

mod process;
#[cfg(feature = "station")]
pub mod station;

use std::collections::BTreeMap;

use serde::de::DeserializeOwned;
use serde_json::Value;
use tracel_job::JobDefinition;

use crate::BoxError;
use crate::job::{IntoJob, Ready, Registered};
use crate::panics::catch_panic;
use crate::scope::{self, ExperimentSink};
use crate::stop::Stop;

/// A program's jobs, by name, and how the program runs the one its command line names: `main`
/// returns [`run`](Self::run).
///
/// The command line is `<program> <job> [<input-json>] [<flags>]`, built from the jobs'
/// definitions as [`command`](tracel_job::command) builds it: one subcommand per job, each with a
/// flag per field of its input, typed by the input schema or the default input, so a job runs
/// without writing JSON. `<program> <job> --help` lists the flags. For a job whose default input is
/// `{"epochs": 10, "optimizer": {"weight_decay": 0.0}}`:
///
/// ```text
/// <program> train --epochs 5 --optimizer.weight-decay 0.01
/// <program> train '{"epochs": 5}'
/// <program> train -c config.json --epochs 5
/// ```
///
/// The input is the `--config` file, the JSON document and the flags, each merged onto the one
/// before, as [`job_input`](tracel_job::job_input) reads it, then merged onto the job's default
/// input. With none, the job runs with its default input, or with `null` when it has none, which
/// a job taking `()` accepts.
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
        I: DeserializeOwned + 'static,
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

    /// The names of the registered jobs, in order.
    fn names(&self) -> Vec<String> {
        self.jobs.keys().cloned().collect()
    }

    /// The job `name`, ready to run with `input` merged onto its default.
    ///
    /// Fails without running anything when no job has that name, or the input does not decode.
    fn prepare(&self, name: &str, input: Value) -> Result<Ready, PrepareError> {
        let job = self.jobs.get(name).ok_or(PrepareError::UnknownJob)?;
        job.prepare(input).map_err(PrepareError::InvalidInput)
    }
}

/// Why a job did not get ready to run.
#[derive(Debug)]
enum PrepareError {
    /// No job has the name given.
    UnknownJob,
    /// The input does not decode as the job's input.
    InvalidInput(serde_json::Error),
}

/// How an adapter launches a job: the request that it stop, the number or id its launcher gave
/// it, and where the experiments it starts are reported.
struct Launch {
    stop: Stop,
    num: Option<String>,
    experiments: ExperimentSink,
}

/// Runs `ready`, the job `launch` launches, on this thread, in a scope of its own, and returns
/// how it ended: a panic is a failure with the panic's message.
fn run(ready: Ready, launch: Launch) -> Result<(), BoxError> {
    let _entered = scope::enter(scope::new(launch.num, launch.stop, launch.experiments));
    catch_panic(ready)
}
