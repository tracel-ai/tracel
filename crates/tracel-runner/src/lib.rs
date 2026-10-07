//! Station runner: serve registered jobs to a Tracel Station job queue.
//!
//! [`StationRunner`] registers experiment jobs with a station and executes the jobs the station
//! dispatches — one at a time, with results reported back as job outcomes. Registration mirrors
//! the command-line and HTTP runners in `tracel-app`:
//!
//! ```ignore
//! use tracel_app::mapper::JsonMapper;
//! use tracel_runner::StationRunner;
//!
//! StationRunner::new("http://localhost:9000")
//!     .name("vision-runner")
//!     .register(train, JsonMapper::with_default(TrainingConfig::default()))
//!     .run()?;
//! ```
//!
//! The runner holds a single connection to the station: a POST to `/v1/runners/events` whose
//! response is a Server-Sent Events stream. The station pushes full job payloads and cancel
//! signals down that stream; presence is the stream itself — when the process dies, the socket
//! closes and the station immediately fails whatever this runner was doing. [`StationRunner::run`]
//! serves forever, reconnecting with backoff when the station is unreachable or restarts.

mod error;
mod infrastructure;
mod runtime;

pub use error::RunnerError;

use std::sync::Arc;

use tracel_app::{IntoJob, JobRegistry};
use tracel_experiment::ExperimentJob;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use infrastructure::StationRunnerClient;
use infrastructure::protocol::RegisterRunner;
use runtime::Executor;

/// A runner process serving jobs to one station.
pub struct StationRunner {
    url: String,
    name: Option<String>,
    jobs: JobRegistry,
}

impl StationRunner {
    /// Create a runner for the station at `url`, the same base URL as `tracel::Target::Station`.
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            name: None,
            jobs: JobRegistry::new(),
        }
    }

    /// Set an optional display label for this runner. Names are not unique — a runner's identity
    /// is its connection, minted by the station per session.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Register an experiment job, decoding its dispatched input with `mapper`.
    ///
    /// Only experiments run on a station, since a station job reports an outcome rather than
    /// outputs:
    ///
    /// ```no_run
    /// use std::sync::Arc;
    ///
    /// use tracel_app::mapper::JsonMapper;
    /// use tracel_experiment::local::LocalExperiments;
    /// use tracel_experiment::{ExperimentRun, Experiments};
    /// use tracel_runner::StationRunner;
    ///
    /// let train = Experiments::new(Arc::new(LocalExperiments::new("./runs")))
    ///     .create("train", |_run: &ExperimentRun, epochs: u32| {
    ///         println!("training for {epochs} epochs");
    ///         Ok(())
    ///     });
    ///
    /// StationRunner::new("http://localhost:8000")
    ///     .register(train, JsonMapper::with_default(10u32))
    ///     .run()?;
    /// # Ok::<(), tracel_runner::RunnerError>(())
    /// ```
    ///
    /// An inference job does not compile:
    ///
    /// ```compile_fail,E0308
    /// use std::sync::Arc;
    ///
    /// use tracel_app::mapper::JsonMapper;
    /// use tracel_inference::{
    ///     InferenceInput, InferenceModule, InferenceOutput, InferenceSession, NoopInferenceProvider,
    /// };
    /// use tracel_runner::StationRunner;
    ///
    /// let echo = InferenceModule::new(Arc::new(NoopInferenceProvider::new())).create(
    ///     "echo",
    ///     |_session: &InferenceSession,
    ///      input: InferenceInput<String>,
    ///      output: InferenceOutput<String>| {
    ///         for text in input {
    ///             let _ = output.write(text);
    ///         }
    ///     },
    /// );
    ///
    /// StationRunner::new("http://localhost:8000")
    ///     .register(echo, JsonMapper::<String>::new())
    ///     .run()?;
    /// # Ok::<(), tracel_runner::RunnerError>(())
    /// ```
    ///
    /// # Panics
    ///
    /// When a job with the same name is already registered.
    pub fn register<I, O, M>(mut self, job: ExperimentJob<I, O>, mapper: M) -> Self
    where
        ExperimentJob<I, O>: IntoJob<M>,
    {
        self.jobs.add(job.into_job(mapper));
        self
    }

    /// Connect to the station, advertise the job definitions, and serve dispatched jobs forever.
    ///
    /// Returns only when the runner cannot start; once serving, connection losses are retried
    /// with backoff and job failures are reported to the station as job outcomes. When
    /// `TRACEL_DESCRIBE` names a path, writes the definitions file there instead and returns
    /// without connecting.
    pub fn run(self) -> Result<(), RunnerError> {
        if self.jobs.describe_from_env("station")? {
            return Ok(());
        }
        let url = url::Url::parse(&self.url).map_err(|source| RunnerError::InvalidUrl {
            url: self.url.clone(),
            source,
        })?;
        if self.jobs.is_empty() {
            return Err(RunnerError::NoJobs);
        }

        let _ = tracing_subscriber::registry()
            .with(tracing_subscriber::fmt::layer())
            .with(tracel_experiment::integration::tracing::tracing_log_layer())
            .with(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
            )
            .try_init();

        let register = RegisterRunner {
            name: self.name,
            jobs: self.jobs.definitions().cloned().collect(),
        };
        let client = StationRunnerClient::new(url);
        let executor = Executor::spawn(Arc::new(self.jobs), Arc::new(client.clone()));
        Err(runtime::serve_forever(client, register, executor))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::Value;
    use tracel_app::mapper::JsonMapper;
    use tracel_experiment::error::ExperimentError;
    use tracel_experiment::{ExperimentProvider, ExperimentRun, Experiments};

    use super::*;

    /// A provider for experiment jobs that are registered, never run.
    struct NeverRuns;

    impl ExperimentProvider for NeverRuns {
        fn create_experiment(
            &self,
            name: String,
            _attributes: HashMap<String, Value>,
        ) -> Result<ExperimentRun, ExperimentError> {
            panic!("experiment '{name}' was not expected to run")
        }
    }

    fn runner_with(url: &str, names: &[&str]) -> StationRunner {
        let experiments = Experiments::new(Arc::new(NeverRuns));
        names.iter().fold(StationRunner::new(url), |runner, name| {
            let train = experiments.create(name, |_run: &ExperimentRun, _epochs: u32| Ok(()));
            runner.register(train, JsonMapper::with_default(10u32))
        })
    }

    #[test]
    fn given_invalid_url_when_running_then_fails_to_start() {
        let result = runner_with("not a url", &["train"]).run();

        assert!(matches!(result, Err(RunnerError::InvalidUrl { .. })));
    }

    #[test]
    fn given_no_jobs_when_running_then_fails_to_start() {
        let result = StationRunner::new("http://localhost:9000").run();

        assert!(matches!(result, Err(RunnerError::NoJobs)));
    }

    #[test]
    #[should_panic(expected = "already registered")]
    fn given_duplicate_job_name_when_registering_then_panics() {
        runner_with("http://localhost:9000", &["train", "train"]);
    }
}
