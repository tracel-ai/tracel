//! Station runner front-end: serve registered jobs to a Tracel Station job queue.
//!
//! [`StationRunner`] registers capability jobs with a station and executes the jobs the station
//! dispatches — one at a time, with results reported back as job outcomes. Registration mirrors
//! the CLI and HTTP server front-ends in `tracel-app`:
//!
//! ```ignore
//! use tracel_runner::StationRunner;
//! use tracel_runner::mapper::JsonInput;
//!
//! StationRunner::new("http://localhost:9000")
//!     .name("vision-runner")
//!     .register(train, JsonInput::with_default(TrainingConfig::default()))
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
mod job;
/// Input mappers that turn a dispatched JSON input into a typed config.
pub mod mapper;
mod runtime;

pub use error::{BoxError, RunnerError};
pub use job::{IntoRunnerJob, JobDefinition, RunnerJob};

use std::collections::HashMap;
use std::sync::Arc;

use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use infrastructure::StationRunnerClient;
use infrastructure::protocol::RegisterRunner;
use runtime::Executor;

/// A runner process serving jobs to one station.
pub struct StationRunner {
    url: String,
    name: Option<String>,
    jobs: HashMap<String, Box<dyn RunnerJob>>,
}

impl StationRunner {
    /// Create a runner for the station at `url` — the same base URL as `Connection::Station`.
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            name: None,
            jobs: HashMap::new(),
        }
    }

    /// Set an optional display label for this runner. Names are not unique — a runner's identity
    /// is its connection, minted by the station per session.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Register a capability job (an experiment), decoding its dispatched input with `mapper`.
    ///
    /// The same call works for any job type that implements [`IntoRunnerJob`].
    pub fn register<T, M>(self, job: T, mapper: M) -> Self
    where
        T: IntoRunnerJob<M>,
    {
        self.job_boxed(job.into_runner_job(mapper))
    }

    /// Register a bespoke [`RunnerJob`]. [`register`](Self::register) builds on this for
    /// capability jobs; use this directly only for a custom job.
    pub fn job<J>(self, job: J) -> Self
    where
        J: RunnerJob + 'static,
    {
        self.job_boxed(Box::new(job))
    }

    fn job_boxed(mut self, job: Box<dyn RunnerJob>) -> Self {
        let name = job.definition().name;
        if self.jobs.contains_key(&name) {
            panic!("job '{name}' is already registered");
        }
        self.jobs.insert(name, job);
        self
    }

    /// Connect to the station, advertise the job manifest, and serve dispatched jobs forever.
    ///
    /// Returns only when the runner cannot start; once serving, connection losses are retried
    /// with backoff and job failures are reported to the station as job outcomes.
    pub fn run(self) -> Result<(), RunnerError> {
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
            jobs: self.jobs.values().map(|job| job.definition()).collect(),
        };
        let client = StationRunnerClient::new(url);
        let executor = Executor::spawn(Arc::new(self.jobs), Arc::new(client.clone()));
        runtime::serve_forever(client, register, executor)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use tracel_experiment::CancelToken;

    use super::*;

    struct FakeJob {
        name: &'static str,
    }

    impl RunnerJob for FakeJob {
        fn definition(&self) -> JobDefinition {
            JobDefinition {
                name: self.name.to_string(),
                description: None,
                input_schema: None,
                input_example: None,
            }
        }

        fn run(&self, _input: &Value, _cancel: CancelToken) -> Result<(), crate::BoxError> {
            Ok(())
        }
    }

    #[test]
    fn given_invalid_url_when_running_then_fails_to_start() {
        let result = StationRunner::new("not a url")
            .job(FakeJob { name: "train" })
            .run();

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
        StationRunner::new("http://localhost:9000")
            .job(FakeJob { name: "train" })
            .job(FakeJob { name: "train" });
    }
}
