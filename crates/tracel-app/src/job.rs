use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tracel_experiment::CancelToken;
use tracel_inference::{OutputWriter, OutputWriterError};
use tracel_job::{JobDefinition, ReportedExperiment};

/// The error a job or a mapper fails with.
pub type BoxError = Box<dyn Error + Send + Sync>;

/// The input a runner hands a job: JSON documents.
///
/// Every job takes either form. A job that takes one input takes the document, or the stream's
/// one document; a job that takes several takes each document in turn.
pub enum JobInput {
    /// One JSON document. `Value::Null` stands for no input.
    Document(Value),
    /// JSON documents taken as they arrive, or why the next one could not be read, which ends
    /// the input.
    Stream(Box<dyn Iterator<Item = Result<Value, BoxError>> + Send>),
}

impl JobInput {
    /// The input of a job that takes one: the document, or the stream's one document, read to the
    /// stream's end. A stream with no document gives `Value::Null`.
    ///
    /// Fails when the stream has a second document, or a document could not be read.
    pub fn one(self) -> Result<Value, BoxError> {
        match self {
            Self::Document(input) => Ok(input),
            Self::Stream(mut inputs) => {
                let input = inputs.next().transpose()?.unwrap_or(Value::Null);
                match inputs.next() {
                    None => Ok(input),
                    Some(Ok(_)) => Err("the job takes one input, but was given more".into()),
                    Some(Err(error)) => Err(error),
                }
            }
        }
    }
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
    /// The context of a job asked to stop once `cancel_token` is cancelled, whose runner links no
    /// experiment.
    pub fn new(cancel_token: CancelToken) -> Self {
        Self {
            cancel_token,
            reporter: ExperimentReporter::default(),
        }
    }

    /// Hands the experiment the job records to `reporter`, through which the runner links it.
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

/// Hands the experiment a job records to its runner.
///
/// A runner that links the experiment a job records, such as in its run report, gives the job a
/// reporter in its [`JobContext`], and a job that records an experiment hands it the experiment
/// once it is created, so the runner links it while the job runs. The default reporter belongs to
/// no runner: it drops what it is given.
#[derive(Clone, Default)]
pub struct ExperimentReporter {
    report: Option<Arc<dyn Fn(ReportedExperiment) + Send + Sync>>,
}

impl ExperimentReporter {
    /// A reporter that hands each experiment to `report`, which links it.
    pub fn new(report: impl Fn(ReportedExperiment) + Send + Sync + 'static) -> Self {
        Self {
            report: Some(Arc::new(report)),
        }
    }

    /// Hands `experiment`, the experiment the job records, to the runner.
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
    /// Cancelling the context's cancel token asks the job to stop: a job that records an experiment
    /// cancels the experiment's cancel token and ends as completed or failed by what its function
    /// returns, and a job that takes several inputs takes no more. A job that records an
    /// experiment hands it to the context's reporter once it is created.
    pub fn run<W>(self, output: W, context: JobContext) -> Result<(), BoxError>
    where
        W: OutputWriter<Value> + Send + Sync + 'static,
    {
        (self.run)(Box::new(output), context)
    }
}

/// A job a runner can run: given its input, it runs to its end, writing its outputs as it goes.
///
/// What a job does as it runs is its own: it may record an experiment, which it hands to the
/// reporter of its [`JobContext`], or answer each of its inputs with outputs. Runners build one
/// with [`IntoJob`] from an experiment or inference job and the
/// [`Mapper`](crate::mapper::Mapper) that decodes its input. Implement it directly only for a job
/// neither covers.
pub trait Job: Send + Sync {
    /// The job's definition.
    fn definition(&self) -> &JobDefinition;

    /// Decodes `input` and returns the job, ready to run with it.
    ///
    /// Fails without running anything when `input` does not decode. A job that takes one input
    /// reads it here, so it waits for a [`JobInput::Stream`] to end.
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

/// An output writer that drops what it is given, for a runner that returns no outputs.
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

    use serde_json::json;

    use super::*;

    fn offline_run() -> ReportedExperiment {
        ReportedExperiment {
            num: Some(3),
            url: None,
            dir: Some("runs/mnist/3".into()),
        }
    }

    fn stream(inputs: Vec<Result<Value, &'static str>>) -> JobInput {
        JobInput::Stream(Box::new(
            inputs
                .into_iter()
                .map(|input| input.map_err(BoxError::from)),
        ))
    }

    #[test]
    fn the_one_input_is_the_document_or_the_streams_one_document() {
        let one = |input: JobInput| input.one().map_err(|error| error.to_string());

        assert_eq!(one(JobInput::Document(json!(3))), Ok(json!(3)));
        assert_eq!(one(stream(vec![Ok(json!(3))])), Ok(json!(3)));
        assert_eq!(one(stream(vec![])), Ok(Value::Null));
        assert_eq!(
            one(stream(vec![Ok(json!(3)), Ok(json!(4))])),
            Err("the job takes one input, but was given more".to_string())
        );
        assert_eq!(
            one(stream(vec![Err("invalid JSON")])),
            Err("invalid JSON".to_string())
        );
        assert_eq!(
            one(stream(vec![Ok(json!(3)), Err("invalid JSON")])),
            Err("invalid JSON".to_string())
        );
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
