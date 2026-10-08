use std::error::Error;
use std::sync::Arc;

use serde_json::Value;
use tracel_experiment::CancelToken;
use tracel_job::{JobDefinition, ReportedExperiment};

/// The error a job or a mapper fails with.
pub type BoxError = Box<dyn Error + Send + Sync>;

/// What a runner gives the job it runs: the token that asks the job to stop, and the reporter the
/// job hands the experiment it records.
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
    run: Box<dyn FnOnce(JobContext) -> Result<(), BoxError> + Send>,
}

impl PreparedJob {
    /// Wraps `run`, which runs the job and stops once the cancel token of the context it is given
    /// is cancelled.
    pub fn new<F>(run: F) -> Self
    where
        F: FnOnce(JobContext) -> Result<(), BoxError> + Send + 'static,
    {
        Self { run: Box::new(run) }
    }

    /// Runs the job in `context`.
    ///
    /// Cancelling the context's cancel token asks the job to stop: a job that records an experiment
    /// cancels the experiment's cancel token and ends as completed or failed by what its function
    /// returns. A job that records an experiment hands it to the context's reporter once it is
    /// created.
    pub fn run(self, context: JobContext) -> Result<(), BoxError> {
        (self.run)(context)
    }
}

/// A job a runner can run: given its input, it runs to its end.
///
/// What a job does as it runs is its own: it may record an experiment, which it hands to the
/// reporter of its [`JobContext`]. Runners build one with [`IntoJob`] from an experiment job and
/// the [`Mapper`](crate::mapper::Mapper) that decodes its input. Implement it directly for any
/// other job.
pub trait Job: Send + Sync {
    /// The job's definition.
    fn definition(&self) -> &JobDefinition;

    /// Decodes `input`, a JSON document or `Value::Null` for no input, and returns the job, ready
    /// to run with it.
    ///
    /// Fails without running anything when `input` does not decode.
    fn prepare(&self, input: Value) -> Result<PreparedJob, BoxError>;
}

/// Turns an experiment job and the mapper that decodes its input into a [`Job`].
///
/// Implemented for `ExperimentJob`, so a runner's `register(job, mapper)` takes one.
pub trait IntoJob<M> {
    /// Builds the job.
    fn into_job(self, mapper: M) -> Box<dyn Job>;
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
