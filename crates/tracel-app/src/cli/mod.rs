mod error;

pub use error::CliError;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use clap::Parser;
use serde_json::Value;
use tracel_inference::{OutputWriter, OutputWriterError};

use crate::{BoxError, IntoJob, Job, JobInput, JobRegistry};

#[derive(Parser)]
#[command(about = "Run a registered job")]
struct Args {
    /// The job to run.
    job: Option<String>,
    /// The job's input as one JSON document; no input when left out.
    input: Option<String>,
}

/// Runs one registered job from the command line: `<job_name> [<input-json>]`.
///
/// The input is one JSON document. Left out, the job runs with no input, which a mapper with a
/// default reads as that default. An inference prints each output as a line of JSON.
#[derive(Default)]
pub struct Cli {
    jobs: JobRegistry,
    default: Option<String>,
}

impl Cli {
    /// A command line with no jobs.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a capability job (an experiment or an inference), decoding its input with
    /// `mapper`.
    ///
    /// # Panics
    ///
    /// When a job with the same name is already registered.
    pub fn register<J, M>(mut self, job: J, mapper: M) -> Self
    where
        J: IntoJob<M>,
    {
        self.jobs.add(job.into_job(mapper));
        self
    }

    /// Registers a job that implements [`Job`] directly.
    ///
    /// # Panics
    ///
    /// When a job with the same name is already registered.
    pub fn job(mut self, job: impl Job + 'static) -> Self {
        self.jobs.add(Box::new(job));
        self
    }

    /// Names the registered job to run when the command line names none.
    pub fn default_job(mut self, name: impl Into<String>) -> Self {
        self.default = Some(name.into());
        self
    }

    /// Runs the job the command-line arguments name.
    ///
    /// When `TRACEL_DESCRIBE` names a path, writes the definitions file there instead and returns
    /// without running a job.
    pub fn run(self) -> Result<(), CliError> {
        if self.jobs.describe_from_env("cli")? {
            return Ok(());
        }
        let args = Args::parse();
        self.dispatch(args.job, args.input)
    }

    fn dispatch(&self, name: Option<String>, input: Option<String>) -> Result<(), CliError> {
        let name = name
            .or_else(|| self.default.clone())
            .ok_or_else(|| CliError::MissingJob {
                available: self.jobs.names(),
            })?;
        let job = self.jobs.get(&name).ok_or_else(|| CliError::UnknownJob {
            name: name.clone(),
            available: self.jobs.names(),
        })?;
        let input = match input {
            Some(input) => {
                serde_json::from_str(&input).map_err(|e| CliError::InvalidInput(e.into()))?
            }
            None => Value::Null,
        };

        let prepared = job
            .prepare(JobInput::Document(input))
            .map_err(CliError::InvalidInput)?;
        let output = Stdout::default();
        let failure = output.failure.clone();
        prepared.run(output).map_err(CliError::JobFailed)?;
        match failure.lock().unwrap().take() {
            Some(error) => Err(CliError::JobFailed(error)),
            None => Ok(()),
        }
    }
}

/// Prints each output as a line of JSON. The first error stops the job and becomes its failure.
#[derive(Default)]
struct Stdout {
    failure: Arc<Mutex<Option<BoxError>>>,
}

impl OutputWriter<Value> for Stdout {
    fn write(&self, output: Value) -> Result<(), OutputWriterError> {
        println!("{output}");
        Ok(())
    }

    fn error(&self, error: BoxError) -> Result<(), OutputWriterError> {
        self.failure.lock().unwrap().get_or_insert(error);
        Err(OutputWriterError::Cancelled)
    }

    fn finish(&self, _duration: Duration) {}
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{JobDefinition, JobKind, PreparedJob};

    #[derive(Clone, Copy)]
    enum Outcome {
        Succeed,
        Fail,
        ReportError,
    }

    /// Takes an object input, or none, and records the input it ran with.
    struct FakeJob {
        definition: JobDefinition,
        outcome: Outcome,
        ran_with: Arc<Mutex<Option<Value>>>,
    }

    impl FakeJob {
        fn new(name: &str, outcome: Outcome) -> Self {
            Self {
                definition: JobDefinition {
                    name: name.to_string(),
                    kind: JobKind::Experiment,
                    description: None,
                    input_schema: None,
                    input_example: None,
                },
                outcome,
                ran_with: Arc::default(),
            }
        }
    }

    impl Job for FakeJob {
        fn definition(&self) -> &JobDefinition {
            &self.definition
        }

        fn prepare(&self, input: JobInput) -> Result<PreparedJob, BoxError> {
            let JobInput::Document(input) = input else {
                return Err("a stream".into());
            };
            if !(input.is_object() || input.is_null()) {
                return Err("not an object".into());
            }
            let ran_with = self.ran_with.clone();
            let outcome = self.outcome;
            Ok(PreparedJob::new(move |output| {
                *ran_with.lock().unwrap() = Some(input);
                match outcome {
                    Outcome::Succeed => Ok(()),
                    Outcome::Fail => Err("boom".into()),
                    Outcome::ReportError => {
                        let _ = output.error("bad output".into());
                        Ok(())
                    }
                }
            }))
        }
    }

    fn cli(job: FakeJob) -> Cli {
        Cli::new()
            .job(job)
            .job(FakeJob::new("evaluate", Outcome::Succeed))
    }

    #[test]
    fn the_named_job_runs_with_its_json_input() {
        let job = FakeJob::new("train", Outcome::Succeed);
        let ran_with = job.ran_with.clone();

        cli(job)
            .dispatch(Some("train".into()), Some(r#"{"epochs": 2}"#.into()))
            .unwrap();

        assert_eq!(*ran_with.lock().unwrap(), Some(json!({"epochs": 2})));
    }

    #[test]
    fn a_missing_input_runs_the_job_with_no_input() {
        let job = FakeJob::new("train", Outcome::Succeed);
        let ran_with = job.ran_with.clone();

        cli(job).dispatch(Some("train".into()), None).unwrap();

        assert_eq!(*ran_with.lock().unwrap(), Some(Value::Null));
    }

    #[test]
    fn the_default_job_runs_when_no_job_is_named() {
        let job = FakeJob::new("train", Outcome::Succeed);
        let ran_with = job.ran_with.clone();

        cli(job).default_job("train").dispatch(None, None).unwrap();

        assert_eq!(*ran_with.lock().unwrap(), Some(Value::Null));
    }

    #[test]
    fn an_unknown_or_missing_job_lists_the_registered_ones() {
        let cli = cli(FakeJob::new("train", Outcome::Succeed));

        let unknown = cli.dispatch(Some("infer".into()), None).unwrap_err();
        let missing = cli.dispatch(None, None).unwrap_err();

        assert!(matches!(
            &unknown,
            CliError::UnknownJob { available, .. } if available == &["evaluate", "train"]
        ));
        assert!(matches!(missing, CliError::MissingJob { .. }));
        assert!(unknown.to_string().contains("evaluate, train"), "{unknown}");
    }

    #[test]
    fn an_input_that_is_not_json_or_does_not_decode_is_invalid() {
        let job = FakeJob::new("train", Outcome::Succeed);
        let ran_with = job.ran_with.clone();
        let cli = cli(job);

        let not_json = cli.dispatch(Some("train".into()), Some("--epochs 2".into()));
        let not_decoded = cli.dispatch(Some("train".into()), Some("[1, 2]".into()));

        assert!(matches!(not_json, Err(CliError::InvalidInput(_))));
        assert!(matches!(not_decoded, Err(CliError::InvalidInput(_))));
        assert!(ran_with.lock().unwrap().is_none());
    }

    #[test]
    fn a_failing_job_or_an_error_output_fails() {
        let failing =
            cli(FakeJob::new("train", Outcome::Fail)).dispatch(Some("train".into()), None);
        let reporting =
            cli(FakeJob::new("train", Outcome::ReportError)).dispatch(Some("train".into()), None);

        assert!(matches!(failing, Err(CliError::JobFailed(e)) if e.to_string() == "boom"));
        assert!(matches!(reporting, Err(CliError::JobFailed(e)) if e.to_string() == "bad output"));
    }

    #[test]
    #[should_panic(expected = "already registered")]
    fn a_job_name_registers_once() {
        cli(FakeJob::new("evaluate", Outcome::Succeed));
    }
}
