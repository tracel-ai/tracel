use std::error::Error;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracel_inference::{OutputWriter, OutputWriterError};

/// The error a job or a mapper fails with.
pub type BoxError = Box<dyn Error + Send + Sync>;

/// What a job runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    /// An experiment: one input, recorded as an experiment run.
    Experiment,
    /// An inference: one or more inputs, each answered with outputs.
    Inference,
}

/// Something a runner can run: its name, kind and description, and the input it takes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobDefinition {
    /// The name that selects the job.
    pub name: String,
    /// What the job runs.
    pub kind: JobKind,
    /// What the job does.
    pub description: Option<String>,
    /// JSON Schema of the job's input, when its mapper provides one.
    pub input_schema: Option<Value>,
    /// An example input, typically the job's default configuration.
    pub input_example: Option<Value>,
}

/// The input a runner hands a job, as JSON.
pub enum JobInput {
    /// One JSON document. `Value::Null` stands for no input.
    Document(Value),
    /// JSON documents taken as they arrive. Only an inference takes a stream.
    Stream(Box<dyn Iterator<Item = Value> + Send>),
}

/// Where a job sends its outputs.
pub type JobOutput = Box<dyn OutputWriter<Value> + Send + Sync>;

/// A job whose input is decoded, ready to run.
pub struct PreparedJob {
    run: Box<dyn FnOnce(JobOutput) -> Result<(), BoxError> + Send>,
}

impl PreparedJob {
    /// Wraps `run`, which runs the job and sends its outputs to the writer it is given.
    pub fn new<F>(run: F) -> Self
    where
        F: FnOnce(JobOutput) -> Result<(), BoxError> + Send + 'static,
    {
        Self { run: Box::new(run) }
    }

    /// Runs the job, sending its outputs to `output`.
    pub fn run<W>(self, output: W) -> Result<(), BoxError>
    where
        W: OutputWriter<Value> + Send + Sync + 'static,
    {
        (self.run)(Box::new(output))
    }
}

/// A job a runner can run.
///
/// Runners build one with [`IntoJob`] from an experiment or inference job and the
/// [`Mapper`](crate::mapper::Mapper) that decodes its input. Implement it directly only for a job
/// neither covers.
pub trait Job: Send + Sync {
    /// The job's definition.
    fn definition(&self) -> &JobDefinition;

    /// Decodes `input` and returns the job, ready to run with it.
    ///
    /// Fails without running anything when `input` does not decode, or when the job does not
    /// take that kind of input.
    fn prepare(&self, input: JobInput) -> Result<PreparedJob, BoxError>;
}

/// Turns a capability job and the mapper that decodes its input into a [`Job`].
///
/// Implemented for `ExperimentJob` and `InferenceJob`, so every runner's `register(job, mapper)`
/// takes either.
pub trait IntoJob<M> {
    /// Builds the job.
    fn into_job(self, mapper: M) -> Box<dyn Job>;
}

/// An output writer that drops what it is given, for runners that return no outputs, such as an
/// experiment's.
pub struct DiscardOutput;

impl OutputWriter<Value> for DiscardOutput {
    fn write(&self, _output: Value) -> Result<(), OutputWriterError> {
        Ok(())
    }

    fn error(&self, _error: BoxError) -> Result<(), OutputWriterError> {
        Ok(())
    }

    fn finish(&self, _duration: Duration) {}
}
