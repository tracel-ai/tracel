use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tracel_experiment::CancelToken;
use tracel_inference::{OutputWriter, OutputWriterError};
use tracel_job::{JobDefinition, ReportedExperiment};

/// The error a job or a mapper fails with.
pub type BoxError = Box<dyn Error + Send + Sync>;

/// The input a runner hands a job, as JSON.
pub enum JobInput {
    /// One JSON document. `Value::Null` stands for no input.
    Document(Value),
    /// JSON documents taken as they arrive. Only an inference takes a stream.
    Stream(Box<dyn Iterator<Item = Value> + Send>),
}

/// Where a job sends its outputs.
pub type JobOutput = Box<dyn OutputWriter<Value> + Send + Sync>;

/// What a runner gives the job it runs, besides where its outputs go: the token that asks the job
/// to stop, and the reporter the job hands the experiment it records.
#[derive(Clone, Default)]
pub struct JobContext {
    cancel_token: CancelToken,
    reporter: ExperimentReporter,
}

impl JobContext {
    /// The context of a job asked to stop once `cancel_token` is cancelled, whose runner keeps no
    /// run report.
    pub fn new(cancel_token: CancelToken) -> Self {
        Self {
            cancel_token,
            reporter: ExperimentReporter::default(),
        }
    }

    /// Hands the experiment the job records to `reporter`, the runner's run report.
    pub fn with_reporter(mut self, reporter: ExperimentReporter) -> Self {
        self.reporter = reporter;
        self
    }

    /// The token cancelled to ask the job to stop.
    pub fn cancel_token(&self) -> &CancelToken {
        &self.cancel_token
    }

    /// The reporter the job hands the experiment it records.
    pub fn reporter(&self) -> &ExperimentReporter {
        &self.reporter
    }
}

/// Hands the experiment a job records to the run report its runner keeps.
///
/// A runner that keeps a run report gives the job a reporter in its [`JobContext`], and an
/// experiment job hands it the experiment once it is created, so the report links it while the
/// job runs. The default reporter belongs to no report: it drops what it is given.
#[derive(Clone, Default)]
pub struct ExperimentReporter {
    report: Option<Arc<dyn Fn(ReportedExperiment) + Send + Sync>>,
}

impl ExperimentReporter {
    /// A reporter that hands each experiment to `report`, which links it to the run report.
    pub fn new(report: impl Fn(ReportedExperiment) + Send + Sync + 'static) -> Self {
        Self {
            report: Some(Arc::new(report)),
        }
    }

    /// Hands `experiment`, the experiment the job records, to the run report.
    pub fn report(&self, experiment: ReportedExperiment) {
        if let Some(report) = &self.report {
            report(experiment);
        }
    }
}

/// A job whose input is decoded, ready to run.
pub struct PreparedJob {
    run: Box<dyn FnOnce(JobOutput, JobContext) -> Result<(), BoxError> + Send>,
}

impl PreparedJob {
    /// Wraps `run`, which runs the job, sends its outputs to the writer it is given, and stops
    /// once the cancel token of the context it is given is cancelled.
    pub fn new<F>(run: F) -> Self
    where
        F: FnOnce(JobOutput, JobContext) -> Result<(), BoxError> + Send + 'static,
    {
        Self { run: Box::new(run) }
    }

    /// Runs the job in `context`, sending its outputs to `output`.
    ///
    /// Cancelling the context's cancel token asks the job to stop: an experiment's run has its
    /// cancel token cancelled and ends as completed or failed by what its function returns, and an
    /// inference takes no more input. An experiment hands the experiment it creates to the
    /// context's reporter.
    pub fn run<W>(self, output: W, context: JobContext) -> Result<(), BoxError>
    where
        W: OutputWriter<Value> + Send + Sync + 'static,
    {
        (self.run)(Box::new(output), context)
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

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    fn offline_run() -> ReportedExperiment {
        ReportedExperiment {
            num: Some(3),
            url: None,
            dir: Some("runs/mnist/3".into()),
        }
    }

    #[test]
    fn a_reporter_hands_each_experiment_to_its_report() {
        let reported = Arc::new(Mutex::new(Vec::new()));
        let reporter = ExperimentReporter::new({
            let reported = reported.clone();
            move |experiment| reported.lock().unwrap().push(experiment)
        });
        let context = JobContext::default().with_reporter(reporter);

        context.clone().reporter().report(offline_run());

        assert_eq!(*reported.lock().unwrap(), [offline_run()]);
    }
}
