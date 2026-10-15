//! Station runner: serve a program's jobs to a Tracel Station job queue (requires the `station`
//! feature).
//!
//! [`StationRunner`] registers the jobs' definitions with a station and runs the jobs the station
//! dispatches, each on a thread of its own, reporting each one's outcome back as completed or
//! failed:
//!
//! ```no_run
//! use tracel_runtime::station::StationRunner;
//! use tracel_runtime::{BoxError, Jobs};
//!
//! fn train(epochs: u32) -> Result<(), BoxError> {
//!     println!("training for {epochs} epochs");
//!     Ok(())
//! }
//!
//! StationRunner::new("http://localhost:8000")
//!     .name("vision-runner")
//!     .run(Jobs::new().job("train", train))?;
//! # Ok::<(), tracel_runtime::station::RunnerError>(())
//! ```
//!
//! The runner holds a single connection to the station: a POST to `/v1/runners/events` whose
//! response is a Server-Sent Events stream. The station pushes full job payloads and cancel
//! signals down that stream; presence is the stream itself — when the process dies, the socket
//! closes and the station immediately fails whatever this runner was doing. [`StationRunner::run`]
//! serves forever, reconnecting with backoff when the station is unreachable or restarts. It logs
//! what it does as `tracing` events and installs no subscriber: the program decides where they go.
//!
//! Each job runs on a thread of its own, in its [`JobScope`](crate::JobScope):
//! [`job_num`](crate::job_num) gives the station's id for it. A `cancel` event is logged, and the
//! job it names runs to completion.

mod error;
mod infrastructure;
mod serve;

pub use error::RunnerError;

use std::sync::Arc;

use serde_json::{Map, Value};

use crate::Jobs;
use infrastructure::StationRunnerClient;
use infrastructure::protocol::{RegisterRunner, RegisteredJob};
use serve::Executor;

/// A runner process serving a program's jobs to one station.
pub struct StationRunner {
    url: String,
    name: Option<String>,
    metadata: Map<String, Value>,
}

impl StationRunner {
    /// A runner for the station at `url`, the same base URL as `tracel::Target::Station`.
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            name: None,
            metadata: Map::new(),
        }
    }

    /// Sets an optional display label for this runner. Names are not unique — a runner's identity
    /// is its connection, minted by the station per session.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Describe this runner to the station with a free-form metadata entry, such as the hardware
    /// it computes on. The station stores the entries as sent and lists them with the runner;
    /// setting a key again replaces its value.
    pub fn metadata(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Connects to the station, advertises the definitions of `jobs`, and serves the jobs it
    /// dispatches forever.
    ///
    /// Returns only when the runner cannot start; once serving, connection losses are retried
    /// with backoff and job failures are reported to the station as job outcomes.
    pub fn run(self, jobs: Jobs) -> Result<(), RunnerError> {
        let url = url::Url::parse(&self.url).map_err(|source| RunnerError::InvalidUrl {
            url: self.url.clone(),
            source,
        })?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(RunnerError::NotHttp { url: self.url });
        }
        if jobs.definitions().next().is_none() {
            return Err(RunnerError::NoJobs);
        }

        let register = RegisterRunner {
            name: self.name,
            metadata: self.metadata,
            jobs: jobs.definitions().map(RegisteredJob::from).collect(),
        };
        let client = StationRunnerClient::new(url).map_err(RunnerError::HttpClient)?;
        let executor = Executor::new(Arc::new(jobs), Arc::new(client.clone()));
        Err(serve::serve_forever(client, register, executor))
    }
}

#[cfg(test)]
mod tests {
    use rstest::{fixture, rstest};

    use super::*;
    use crate::BoxError;

    fn train(_epochs: u32) -> Result<(), BoxError> {
        Ok(())
    }

    #[fixture]
    fn jobs() -> Jobs {
        Jobs::new().job("train", train)
    }

    #[rstest]
    fn a_url_that_does_not_parse_does_not_start_the_runner(jobs: Jobs) {
        let given = "not a url";

        let failed = StationRunner::new(given).run(jobs);

        assert!(matches!(failed, Err(RunnerError::InvalidUrl { url, .. }) if url == given));
    }

    #[rstest]
    #[case::a_host_and_port("localhost:8000")]
    #[case::of_another_scheme("ftp://station")]
    fn a_url_that_is_not_http_does_not_start_the_runner(jobs: Jobs, #[case] given: &str) {
        let failed = StationRunner::new(given).run(jobs);

        assert!(matches!(failed, Err(RunnerError::NotHttp { url }) if url == given));
    }

    #[test]
    fn a_runner_with_no_jobs_does_not_start() {
        let failed = StationRunner::new("http://localhost:9000").run(Jobs::new());

        assert!(matches!(failed, Err(RunnerError::NoJobs)));
    }
}
