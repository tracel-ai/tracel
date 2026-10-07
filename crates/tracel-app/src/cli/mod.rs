mod error;
mod signal;

use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use clap::Parser;
use serde_json::Value;
use tracel_experiment::CancelToken;
use tracel_inference::{OutputWriter, OutputWriterError};

use crate::{BoxError, IntoJob, Job, JobInput, JobRegistry};
use error::CliError;

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
///
/// [`run`](Self::run) returns the exit code for the process, so `main` returns it:
///
/// ```no_run
/// use std::process::ExitCode;
/// use std::sync::Arc;
///
/// use tracel_app::cli::Cli;
/// use tracel_app::mapper::JsonMapper;
/// use tracel_experiment::local::LocalExperiments;
/// use tracel_experiment::{ExperimentRun, Experiments};
///
/// fn main() -> ExitCode {
///     let train = Experiments::new(Arc::new(LocalExperiments::new("./runs")))
///         .create("train", |_run: &ExperimentRun, epochs: u32| {
///             println!("training for {epochs} epochs");
///             Ok(())
///         });
///
///     Cli::new()
///         .register(train, JsonMapper::with_default(10u32))
///         .run()
/// }
/// ```
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

    /// Runs the job the command-line arguments name, and returns the exit code for the process.
    ///
    /// | Exit code | Meaning |
    /// | --- | --- |
    /// | 0 | The job completed, or the definitions file was written |
    /// | 1 | The job failed, or the definitions file could not be written |
    /// | 2 | No job or an unknown job is named, or the input is not JSON or does not decode |
    /// | 130 | The job was cancelled |
    ///
    /// Why it did not complete is printed to stderr, with the registered job names when no job or
    /// an unknown one is named. When `TRACEL_DESCRIBE` names a path, writes the definitions file
    /// there instead of running a job.
    ///
    /// SIGTERM, SIGINT or SIGHUP, or Ctrl-C, Ctrl-Break or closing the console on Windows,
    /// cancels the job: an experiment's run is cancelled, and it ends as cancelled once its
    /// function returns. A second signal ends the process at once, with exit code 130. Launchers
    /// send SIGKILL after a 30-second grace period.
    pub fn run(self) -> ExitCode {
        match self.execute() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("error: {error}");
                ExitCode::from(error.exit_code())
            }
        }
    }

    fn execute(self) -> Result<(), CliError> {
        if self.jobs.describe_from_env("cli")? {
            return Ok(());
        }
        let args = Args::parse();
        let cancel_token = CancelToken::new();
        signal::cancel_on_termination(cancel_token.clone());
        self.dispatch(args.job, args.input, cancel_token)
    }

    fn dispatch(
        &self,
        name: Option<String>,
        input: Option<String>,
        cancel_token: CancelToken,
    ) -> Result<(), CliError> {
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
        let ran = catch_unwind(AssertUnwindSafe(|| {
            prepared.run(output, cancel_token.clone())
        }));

        if cancel_token.is_cancelled() {
            return Err(CliError::Cancelled);
        }
        match ran {
            Ok(Ok(())) => match failure.lock().unwrap().take() {
                Some(error) => Err(CliError::JobFailed(error)),
                None => Ok(()),
            },
            Ok(Err(error)) => Err(CliError::JobFailed(error)),
            Err(panic) => Err(CliError::JobFailed(
                format!("the job panicked: {}", panic_message(panic.as_ref())).into(),
            )),
        }
    }
}

fn panic_message(panic: &(dyn Any + Send)) -> &str {
    if let Some(message) = panic.downcast_ref::<&str>() {
        message
    } else if let Some(message) = panic.downcast_ref::<String>() {
        message
    } else {
        "unknown panic"
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
        Panic,
        Cancel,
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
            Ok(PreparedJob::new(move |output, cancel_token| {
                *ran_with.lock().unwrap() = Some(input);
                match outcome {
                    Outcome::Succeed => Ok(()),
                    Outcome::Fail => Err("boom".into()),
                    Outcome::ReportError => {
                        let _ = output.error("bad output".into());
                        Ok(())
                    }
                    Outcome::Panic => panic!("kernel exploded"),
                    Outcome::Cancel => {
                        cancel_token.cancel();
                        Err("interrupted".into())
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

    /// Runs the job `name` names with `input`, as the command line `<name> <input>`.
    fn run(cli: &Cli, name: Option<&str>, input: Option<&str>) -> Result<(), CliError> {
        cli.dispatch(
            name.map(str::to_string),
            input.map(str::to_string),
            CancelToken::new(),
        )
    }

    #[test]
    fn the_named_job_runs_with_its_json_input() {
        let job = FakeJob::new("train", Outcome::Succeed);
        let ran_with = job.ran_with.clone();

        run(&cli(job), Some("train"), Some(r#"{"epochs": 2}"#)).unwrap();

        assert_eq!(*ran_with.lock().unwrap(), Some(json!({"epochs": 2})));
    }

    #[test]
    fn a_missing_input_runs_the_job_with_no_input() {
        let job = FakeJob::new("train", Outcome::Succeed);
        let ran_with = job.ran_with.clone();

        run(&cli(job), Some("train"), None).unwrap();

        assert_eq!(*ran_with.lock().unwrap(), Some(Value::Null));
    }

    #[test]
    fn the_default_job_runs_when_no_job_is_named() {
        let job = FakeJob::new("train", Outcome::Succeed);
        let ran_with = job.ran_with.clone();

        run(&cli(job).default_job("train"), None, None).unwrap();

        assert_eq!(*ran_with.lock().unwrap(), Some(Value::Null));
    }

    #[test]
    fn an_unknown_or_missing_job_lists_the_registered_ones() {
        let cli = cli(FakeJob::new("train", Outcome::Succeed));

        let unknown = run(&cli, Some("infer"), None).unwrap_err();
        let missing = run(&cli, None, None).unwrap_err();

        assert!(matches!(
            &unknown,
            CliError::UnknownJob { available, .. } if available == &["evaluate", "train"]
        ));
        assert!(matches!(missing, CliError::MissingJob { .. }));
        assert!(unknown.to_string().contains("evaluate, train"), "{unknown}");
        assert!(missing.to_string().contains("evaluate, train"), "{missing}");
    }

    #[test]
    fn an_input_that_is_not_json_or_does_not_decode_is_invalid() {
        let job = FakeJob::new("train", Outcome::Succeed);
        let ran_with = job.ran_with.clone();
        let cli = cli(job);

        let not_json = run(&cli, Some("train"), Some("--epochs 2"));
        let not_decoded = run(&cli, Some("train"), Some("[1, 2]"));

        assert!(matches!(not_json, Err(CliError::InvalidInput(_))));
        assert!(matches!(not_decoded, Err(CliError::InvalidInput(_))));
        assert!(ran_with.lock().unwrap().is_none());
    }

    #[test]
    fn a_failing_job_or_an_error_output_fails() {
        let failing = run(
            &cli(FakeJob::new("train", Outcome::Fail)),
            Some("train"),
            None,
        );
        let reporting = run(
            &cli(FakeJob::new("train", Outcome::ReportError)),
            Some("train"),
            None,
        );

        assert!(matches!(failing, Err(CliError::JobFailed(e)) if e.to_string() == "boom"));
        assert!(matches!(reporting, Err(CliError::JobFailed(e)) if e.to_string() == "bad output"));
    }

    #[test]
    fn a_panicking_job_fails() {
        let panicking = run(
            &cli(FakeJob::new("train", Outcome::Panic)),
            Some("train"),
            None,
        );

        assert!(matches!(
            panicking,
            Err(CliError::JobFailed(e)) if e.to_string().contains("kernel exploded")
        ));
    }

    #[test]
    fn a_job_whose_token_is_cancelled_is_cancelled_even_when_it_fails() {
        let cancelled = run(
            &cli(FakeJob::new("train", Outcome::Cancel)),
            Some("train"),
            None,
        );

        assert!(matches!(cancelled, Err(CliError::Cancelled)));
    }

    #[test]
    fn each_outcome_has_its_exit_code() {
        let cli = cli(FakeJob::new("train", Outcome::Succeed));
        let exit_code = |outcome: Result<(), CliError>| outcome.unwrap_err().exit_code();

        assert_eq!(exit_code(run(&cli, Some("infer"), None)), 2);
        assert_eq!(exit_code(run(&cli, None, None)), 2);
        assert_eq!(exit_code(run(&cli, Some("train"), Some("{"))), 2);
        assert_eq!(exit_code(run(&cli, Some("train"), Some("[1, 2]"))), 2);
        for (outcome, code) in [
            (Outcome::Fail, 1),
            (Outcome::ReportError, 1),
            (Outcome::Panic, 1),
            (Outcome::Cancel, 130),
        ] {
            let cli = self::cli(FakeJob::new("train", outcome));
            assert_eq!(exit_code(run(&cli, Some("train"), None)), code);
        }
    }

    #[test]
    #[should_panic(expected = "already registered")]
    fn a_job_name_registers_once() {
        cli(FakeJob::new("evaluate", Outcome::Succeed));
    }
}
