use serde::Serialize;
use serde_json::Value;

use tracel_experiment::{CancelToken, ExperimentJob};

use crate::error::BoxError;
use crate::mapper::InputMapper;

/// A manifest entry describing one job this runner offers, sent verbatim to the station on
/// registration. The station validates submissions against the connected runners' manifests.
#[derive(Debug, Clone, Serialize)]
pub struct JobDefinition {
    /// Name clients use to queue this job.
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// JSON Schema of the job input; reserved for station-side validation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<Value>,
    /// Example input, typically the job's default configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_example: Option<Value>,
}

/// A capability the runner can execute from a dispatched JSON input.
///
/// This is the runner's local trait. You rarely implement it directly —
/// [`StationRunner::register`](crate::StationRunner::register) accepts any capability job via
/// [`IntoRunnerJob`]. Implement it only for a bespoke job.
pub trait RunnerJob: Send + Sync {
    /// The manifest entry advertised to the station.
    fn definition(&self) -> JobDefinition;
    /// Run the job with the dispatched input. `cancel` fires when the station requests
    /// cancellation; observing it is cooperative.
    fn run(&self, input: &Value, cancel: CancelToken) -> Result<(), BoxError>;
}

/// Turns a capability job plus an input mapper into a [`RunnerJob`].
///
/// Implemented for `ExperimentJob`, so `StationRunner::register(job, mapper)` mirrors the CLI and
/// server front-ends. A new capability becomes runner-registrable by implementing this for its job.
pub trait IntoRunnerJob<M> {
    fn into_runner_job(self, mapper: M) -> Box<dyn RunnerJob>;
}

/// Runs an [`ExperimentJob`] on the runner: decode the input, run the experiment to completion
/// with the station's cancel signal linked into the run.
struct ExperimentRunnerJob<I, O, M> {
    job: ExperimentJob<I, O>,
    mapper: M,
}

impl<I, O, M> RunnerJob for ExperimentRunnerJob<I, O, M>
where
    I: Send + 'static,
    O: 'static,
    M: InputMapper<I> + Send + Sync,
{
    fn definition(&self) -> JobDefinition {
        JobDefinition {
            name: self.job.name().to_string(),
            description: None,
            input_schema: None,
            input_example: self.mapper.example(),
        }
    }

    fn run(&self, input: &Value, cancel: CancelToken) -> Result<(), BoxError> {
        let input = self
            .mapper
            .map(input)
            .map_err(|e| BoxError::from(format!("invalid input: {e}")))?;
        self.job.run_with_cancellation(input, cancel).map(|_| ())
    }
}

impl<I, O, M> IntoRunnerJob<M> for ExperimentJob<I, O>
where
    I: Send + 'static,
    O: 'static,
    M: InputMapper<I> + Send + Sync + 'static,
{
    fn into_runner_job(self, mapper: M) -> Box<dyn RunnerJob> {
        Box::new(ExperimentRunnerJob { job: self, mapper })
    }
}
