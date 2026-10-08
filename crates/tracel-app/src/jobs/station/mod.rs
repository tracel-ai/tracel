//! Station runner: serve a program's jobs to a Tracel Station job queue (requires the `station`
//! feature).
//!
//! [`StationRunner`] registers the jobs' definitions with a station and runs the jobs the station
//! dispatches, each on a thread of its own, reporting each one's outcome back as completed or
//! failed:
//!
//! ```no_run
//! use tracel_app::station::StationRunner;
//! use tracel_app::{BoxError, Jobs};
//!
//! fn train(epochs: u32) -> Result<(), BoxError> {
//!     println!("training for {epochs} epochs");
//!     Ok(())
//! }
//!
//! StationRunner::new("http://localhost:8000")
//!     .name("vision-runner")
//!     .run(Jobs::new().job("train", train))?;
//! # Ok::<(), tracel_app::station::RunnerError>(())
//! ```
//!
//! The runner holds a single connection to the station: a POST to `/v1/runners/events` whose
//! response is a Server-Sent Events stream. The station pushes full job payloads and cancel
//! signals down that stream; presence is the stream itself — when the process dies, the socket
//! closes and the station immediately fails whatever this runner was doing. [`StationRunner::run`]
//! serves forever, reconnecting with backoff when the station is unreachable or restarts.
//!
//! Each job runs on a thread of its own, in its [`JobScope`](crate::JobScope):
//! [`job_num`](crate::job_num) gives the station's id for it, and the experiments it reports are
//! logged to stderr.

mod error;
mod infrastructure;
mod serve;

pub use error::RunnerError;

use std::sync::Arc;

use serde_json::{Map, Value};

use crate::Jobs;
use infrastructure::StationRunnerClient;
use infrastructure::protocol::RegisterRunner;
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
        if jobs.definitions().next().is_none() {
            return Err(RunnerError::NoJobs);
        }

        let register = RegisterRunner {
            name: self.name,
            metadata: self.metadata,
            jobs: jobs.definitions().cloned().collect(),
        };
        let client = StationRunnerClient::new(url);
        let executor = Executor::new(Arc::new(jobs), Arc::new(client.clone()));
        Err(serve::serve_forever(client, register, executor))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BoxError;

    fn train(_epochs: u32) -> Result<(), BoxError> {
        Ok(())
    }

    #[test]
    fn given_invalid_url_when_running_then_fails_to_start() {
        let result = StationRunner::new("not a url").run(Jobs::new().job("train", train));

        assert!(matches!(result, Err(RunnerError::InvalidUrl { .. })));
    }

    #[test]
    fn given_no_jobs_when_running_then_fails_to_start() {
        let result = StationRunner::new("http://localhost:9000").run(Jobs::new());

        assert!(matches!(result, Err(RunnerError::NoJobs)));
    }
}
