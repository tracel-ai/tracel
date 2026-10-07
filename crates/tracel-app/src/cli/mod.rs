mod error;
mod report;
mod signal;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use clap::Command;
use clap::error::{ContextKind, ContextValue, ErrorKind};
use clap_complete::Shell;
use serde_json::Value;
use tracel_experiment::CancelToken;
use tracel_inference::{OutputWriter, OutputWriterError};
use tracel_job::{command, completions, job_input};

use crate::panics::catch_panic;
use crate::{BoxError, IntoJob, Job, JobContext, JobInput, JobRegistry};
use error::CliError;
use report::ReportFile;

/// Runs one registered job from the command line: `<job_name> [<input-json>] [<flags>]`.
///
/// The command line is built from the jobs' definitions, as [`command`] builds it: one
/// subcommand per job, each with a flag per field of its input, typed by the input schema or the
/// example input, so a job runs without writing JSON. `<program> <job> --help` lists the flags.
/// For a job whose example input is `{"epochs": 10, "optimizer": {"weight_decay": 0.0}}`:
///
/// ```text
/// <program> train --epochs 5 --optimizer.weight-decay 0.01
/// <program> train '{"epochs": 5}'
/// <program> train -c config.json --epochs 5
/// ```
///
/// The input is the `--config` file, the JSON document and the flags, each merged onto the one
/// before, as [`job_input`] reads it. With none, the job runs with no input, which a mapper with
/// a default reads as that default. Each output the job writes is printed as a line of JSON.
///
/// When `TRACEL_REPORT_FILE` names a path, the job's [`RunReport`](tracel_job::RunReport) is
/// written there when the job starts, again when the job records an experiment, and again when it
/// ends, as completed or failed by what the job returns. None is written for a command line or an
/// input that is rejected.
///
/// `<program> --completions <SHELL>` prints the program's completion script for `bash`,
/// `elvish`, `fish`, `powershell` or `zsh`, and with [`version`](Self::version),
/// `<program> --version` prints the program's name and version.
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
///         .version(env!("CARGO_PKG_VERSION"))
///         .register(train, JsonMapper::with_default(10u32))
///         .run()
/// }
/// ```
#[derive(Default)]
pub struct Cli {
    jobs: JobRegistry,
    default: Option<String>,
    version: Option<String>,
}

/// What a command line asks for.
enum Request {
    /// Run the job `job` with `input`.
    Run { job: String, input: Value },
    /// Print the completion script for `shell`.
    Completions(Shell),
    /// Print the help or the version, which clap gives as an error that is not one.
    Print(clap::Error),
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

    /// Names the registered job to run, with no input, when the command line names none.
    pub fn default_job(mut self, name: impl Into<String>) -> Self {
        self.default = Some(name.into());
        self
    }

    /// Gives the program's version, such as `env!("CARGO_PKG_VERSION")`, which
    /// `<program> --version` prints after the program's name. Without one, the command line
    /// has no `--version`.
    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    /// Runs the job the command-line arguments name, and returns the exit code for the process.
    ///
    /// | Exit code | Meaning |
    /// | --- | --- |
    /// | 0 | The job completed, the help, version or completion script was printed, or the definitions file was written |
    /// | 1 | The job failed, or the definitions file or the run report could not be written |
    /// | 2 | No job or an unknown job is named, a flag or the input is unusable, or the input does not decode |
    /// | 130 | The job was asked to stop |
    ///
    /// Why it did not complete is printed to stderr, with the registered job names when no job or
    /// an unknown one is named. When `TRACEL_DESCRIBE` names a path, writes the definitions file
    /// there instead of running a job.
    ///
    /// SIGTERM, SIGINT or SIGHUP, or Ctrl-C, Ctrl-Break or closing the console on Windows, asks
    /// the job to stop: it cancels the cancel token of an experiment's run, which ends as completed
    /// or failed by what its function returns, and the process exits with code 130. A second
    /// signal ends the process at once, also with exit code 130. Launchers send SIGKILL after a
    /// 30-second grace period.
    pub fn run(self) -> ExitCode {
        match self.execute() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                error.print();
                ExitCode::from(error.exit_code())
            }
        }
    }

    fn execute(self) -> Result<(), CliError> {
        if self.jobs.describe_from_env("cli")? {
            return Ok(());
        }
        let mut command = self.command_line(program_name());
        match self.parse(&mut command, std::env::args_os())? {
            Request::Run { job, input } => {
                let cancel_token = CancelToken::new();
                signal::cancel_on_termination(cancel_token.clone());
                self.dispatch(&job, input, cancel_token, report::path_from_env())
            }
            Request::Completions(shell) => {
                let name = command.get_name().to_string();
                clap_complete::generate(shell, &mut command, name, &mut std::io::stdout());
                Ok(())
            }
            Request::Print(text) => {
                let _ = text.print();
                Ok(())
            }
        }
    }

    /// The command line of the program `name`: its jobs' [`command`], with `--version` when the
    /// program gives its version.
    fn command_line(&self, name: impl Into<String>) -> Command {
        let command = command(name, self.jobs.definitions());
        match &self.version {
            Some(version) => command.version(version.clone()),
            None => command,
        }
    }

    /// Reads what the command-line arguments `args`, the program's name first, ask for.
    fn parse<I, T>(&self, command: &mut Command, args: I) -> Result<Request, CliError>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        let matches = match command.try_get_matches_from_mut(args) {
            Ok(matches) => matches,
            Err(error) if !error.use_stderr() => return Ok(Request::Print(error)),
            Err(error) if error.kind() == ErrorKind::InvalidSubcommand => {
                let name = match error.get(ContextKind::InvalidSubcommand) {
                    Some(ContextValue::String(name)) => name.clone(),
                    _ => String::new(),
                };
                return Err(CliError::UnknownJob {
                    name,
                    available: self.jobs.names(),
                });
            }
            Err(error) => return Err(CliError::Usage(error)),
        };
        if let Some(shell) = completions(&matches) {
            return Ok(Request::Completions(shell));
        }
        match (matches.subcommand(), &self.default) {
            (Some((job, matches)), _) => Ok(Request::Run {
                job: job.to_string(),
                input: job_input(matches),
            }),
            (None, Some(job)) => Ok(Request::Run {
                job: job.clone(),
                input: Value::Null,
            }),
            (None, None) => Err(CliError::MissingJob {
                available: self.jobs.names(),
            }),
        }
    }

    /// Runs the job `name` with `input`, asked to stop once `cancel_token` is cancelled, keeping
    /// its run report at `report_path` when there is one.
    fn dispatch(
        &self,
        name: &str,
        input: Value,
        cancel_token: CancelToken,
        report_path: Option<PathBuf>,
    ) -> Result<(), CliError> {
        let job = self.jobs.get(name).ok_or_else(|| CliError::UnknownJob {
            name: name.to_string(),
            available: self.jobs.names(),
        })?;

        let prepared = job
            .prepare(JobInput::Document(input))
            .map_err(CliError::InvalidInput)?;
        let report = report_path
            .map(|path| ReportFile::start(path, name))
            .transpose()?;
        let mut context = JobContext::new(cancel_token.clone());
        if let Some(report) = &report {
            context = context.with_reporter(report.experiment_reporter());
        }
        let output = Stdout::default();
        let failure = output.failure.clone();
        let outcome = catch_panic(|| prepared.run(output, context))
            .and_then(|()| failure.lock().unwrap().take().map_or(Ok(()), Err));

        if let Some(report) = report
            && let Err(error) = report.finish(&outcome)
        {
            eprintln!("warning: {error}");
        }
        if cancel_token.is_cancelled() {
            return Err(CliError::Stopped);
        }
        outcome.map_err(CliError::JobFailed)
    }
}

/// The name the program was run as, without its extension, or `job` when it has none.
fn program_name() -> String {
    std::env::args_os()
        .next()
        .as_deref()
        .map(Path::new)
        .and_then(Path::file_stem)
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "job".to_string())
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
    use tracel_experiment::local::LocalExperiments;
    use tracel_experiment::{ExperimentRun, Experiments};
    use tracel_inference::{
        InferenceInput, InferenceModule, InferenceOutput, InferenceSession, NoopInferenceProvider,
    };
    use tracel_job::{JobDefinition, job_command};

    use super::*;
    use crate::PreparedJob;
    use crate::mapper::{ClapMapper, JsonMapper, Mapper, PresetMapper};

    #[derive(Clone, Copy)]
    enum Outcome {
        Succeed,
        Fail,
        ReportError,
        Panic,
        /// Asks itself to stop, then fails.
        Cancel,
        /// Asks itself to stop, then succeeds.
        Stop,
    }

    /// Takes an object input, or none, and records the input it ran with.
    struct FakeJob {
        definition: JobDefinition,
        outcome: Outcome,
        ran_with: Arc<Mutex<Option<Value>>>,
    }

    /// The definition of a job named `name` whose input has `schema` and `example`.
    fn definition(name: &str, schema: Option<Value>, example: Option<Value>) -> JobDefinition {
        JobDefinition {
            name: name.to_string(),
            description: None,
            input_schema: schema,
            input_example: example,
        }
    }

    impl FakeJob {
        fn new(name: &str, outcome: Outcome) -> Self {
            Self {
                definition: definition(name, None, None),
                outcome,
                ran_with: Arc::default(),
            }
        }

        /// A job that succeeds and lists `example` as its example input.
        fn with_example(name: &str, example: Value) -> Self {
            let mut job = Self::new(name, Outcome::Succeed);
            job.definition.input_example = Some(example);
            job
        }
    }

    impl Job for FakeJob {
        fn definition(&self) -> &JobDefinition {
            &self.definition
        }

        fn prepare(&self, input: JobInput) -> Result<PreparedJob, BoxError> {
            let input = input.one()?;
            if !(input.is_object() || input.is_null()) {
                return Err("not an object".into());
            }
            let ran_with = self.ran_with.clone();
            let outcome = self.outcome;
            Ok(PreparedJob::new(move |output, context| {
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
                        context.cancel_token().cancel();
                        Err("interrupted".into())
                    }
                    Outcome::Stop => {
                        context.cancel_token().cancel();
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

    /// What the command line `program <args>` asks of `cli`.
    fn parse(cli: &Cli, args: &[&str]) -> Result<Request, CliError> {
        let mut command = cli.command_line("program");
        cli.parse(&mut command, ["program"].iter().chain(args))
    }

    /// Runs the command line `program <args>` as [`Cli::execute`] does, printing nothing.
    fn run(cli: &Cli, args: &[&str]) -> Result<(), CliError> {
        run_reporting(cli, args, None)
    }

    /// Runs the command line `program <args>` as [`run`] does, keeping the run report at
    /// `report_path` when there is one.
    fn run_reporting(cli: &Cli, args: &[&str], report_path: Option<&Path>) -> Result<(), CliError> {
        match parse(cli, args)? {
            Request::Run { job, input } => cli.dispatch(
                &job,
                input,
                CancelToken::new(),
                report_path.map(Path::to_path_buf),
            ),
            Request::Completions(_) | Request::Print(_) => Ok(()),
        }
    }

    #[test]
    fn the_named_job_runs_with_its_json_input() {
        let job = FakeJob::new("train", Outcome::Succeed);
        let ran_with = job.ran_with.clone();

        run(&cli(job), &["train", r#"{"epochs": 2, "tag": null}"#]).unwrap();

        assert_eq!(
            *ran_with.lock().unwrap(),
            Some(json!({"epochs": 2, "tag": null}))
        );
    }

    #[test]
    fn a_missing_input_runs_the_job_with_no_input() {
        let job = FakeJob::new("train", Outcome::Succeed);
        let ran_with = job.ran_with.clone();

        run(&cli(job), &["train"]).unwrap();

        assert_eq!(*ran_with.lock().unwrap(), Some(Value::Null));
    }

    #[test]
    fn flags_set_the_fields_of_the_input() {
        let job = FakeJob::with_example(
            "train",
            json!({"epochs": 10, "optimizer": {"weight_decay": 0.0}}),
        );
        let ran_with = job.ran_with.clone();

        run(
            &cli(job),
            &[
                "train",
                "--optimizer.weight-decay",
                "0.01",
                r#"{"epochs": 2}"#,
            ],
        )
        .unwrap();

        assert_eq!(
            *ran_with.lock().unwrap(),
            Some(json!({"epochs": 2, "optimizer": {"weight_decay": 0.01}}))
        );
    }

    #[test]
    fn a_job_that_parses_its_own_arguments_or_names_a_preset_takes_a_json_string() {
        #[derive(clap::Parser)]
        struct Args {
            #[arg(long, default_value_t = 1)]
            epochs: u32,
        }

        let dir = tempfile::tempdir().unwrap();
        let experiments = Experiments::new(Arc::new(LocalExperiments::new(dir.path())));
        let ran = Arc::new(Mutex::new(Vec::new()));
        let parsed = experiments.create("parsed", {
            let ran = ran.clone();
            move |_run: &ExperimentRun, args: Args| {
                ran.lock().unwrap().push(args.epochs);
                Ok(())
            }
        });
        let preset = experiments.create("preset", {
            let ran = ran.clone();
            move |_run: &ExperimentRun, epochs: u32| {
                ran.lock().unwrap().push(epochs);
                Ok(())
            }
        });
        let cli = Cli::new()
            .register(parsed, ClapMapper::<Args>::new())
            .register(
                preset,
                PresetMapper::new().preset("short", 2).preset("long", 20),
            );

        run(&cli, &["parsed", r#""--epochs 3""#]).unwrap();
        run(&cli, &["preset", r#""long""#]).unwrap();
        let flags = run(&cli, &["parsed", "--epochs", "3"]);

        assert_eq!(*ran.lock().unwrap(), [3, 20]);
        assert!(matches!(flags, Err(CliError::Usage(_))), "{flags:?}");
    }

    #[test]
    fn the_default_job_runs_when_no_job_is_named() {
        let job = FakeJob::new("train", Outcome::Succeed);
        let ran_with = job.ran_with.clone();

        run(&cli(job).default_job("train"), &[]).unwrap();

        assert_eq!(*ran_with.lock().unwrap(), Some(Value::Null));
    }

    #[test]
    fn an_unknown_or_missing_job_lists_the_registered_ones() {
        let cli = cli(FakeJob::new("train", Outcome::Succeed));

        let unknown = run(&cli, &["infer"]).unwrap_err();
        let missing = run(&cli, &[]).unwrap_err();
        let unknown_default = run(&cli.default_job("infer"), &[]).unwrap_err();

        for unknown in [&unknown, &unknown_default] {
            assert!(
                matches!(
                    unknown,
                    CliError::UnknownJob { name, available }
                        if name == "infer" && available == &["evaluate", "train"]
                ),
                "{unknown:?}"
            );
            assert!(unknown.to_string().contains("evaluate, train"), "{unknown}");
        }
        assert!(matches!(missing, CliError::MissingJob { .. }));
        assert!(missing.to_string().contains("evaluate, train"), "{missing}");
    }

    #[test]
    fn an_unusable_command_line_or_an_input_that_does_not_decode_is_invalid() {
        let job = FakeJob::with_example("train", json!({"epochs": 10}));
        let ran_with = job.ran_with.clone();
        let cli = cli(job);

        let not_json = run(&cli, &["train", "{"]);
        let not_an_integer = run(&cli, &["train", "--epochs", "ten"]);
        let unknown_flag = run(&cli, &["train", "--epoch", "2"]);
        let not_decoded = run(&cli, &["train", "[1, 2]"]);

        for usage in [not_json, not_an_integer, unknown_flag] {
            assert!(matches!(usage, Err(CliError::Usage(_))), "{usage:?}");
        }
        assert!(matches!(not_decoded, Err(CliError::InvalidInput(_))));
        assert!(ran_with.lock().unwrap().is_none());
    }

    #[test]
    fn help_and_completions_are_printed_instead_of_running_a_job() {
        let cli = cli(FakeJob::with_example("train", json!({"epochs": 10})));

        for args in [&["--help"][..], &["train", "--help"], &["train", "-h"]] {
            assert!(
                matches!(parse(&cli, args), Ok(Request::Print(help)) if help.exit_code() == 0),
                "{args:?}"
            );
        }
        assert!(matches!(
            parse(&cli, &["--completions", "bash"]),
            Ok(Request::Completions(Shell::Bash))
        ));
        assert!(matches!(
            parse(&cli, &["--completions", "bash", "train"]),
            Err(CliError::Usage(_))
        ));
    }

    #[test]
    fn the_version_the_program_gives_is_printed_with_its_name() {
        let cli = cli(FakeJob::new("train", Outcome::Succeed)).version("1.2.3");

        let Ok(Request::Print(version)) = parse(&cli, &["--version"]) else {
            panic!("--version printed nothing");
        };

        assert_eq!(version.kind(), ErrorKind::DisplayVersion);
        assert!(!version.use_stderr());
        assert_eq!(version.exit_code(), 0);
        assert_eq!(version.to_string(), "program 1.2.3\n");
    }

    #[test]
    fn without_a_version_there_is_no_version_flag() {
        let cli = cli(FakeJob::new("train", Outcome::Succeed));

        let unoffered = parse(&cli, &["--version"]);

        assert!(
            matches!(
                &unoffered,
                Err(CliError::Usage(error)) if error.kind() == ErrorKind::UnknownArgument
            ),
            "{:?}",
            unoffered.as_ref().err()
        );
        assert_eq!(unoffered.err().map(|error| error.exit_code()), Some(2));
    }

    #[test]
    fn a_failing_job_or_an_error_output_fails() {
        let failing = run(&cli(FakeJob::new("train", Outcome::Fail)), &["train"]);
        let reporting = run(
            &cli(FakeJob::new("train", Outcome::ReportError)),
            &["train"],
        );

        assert!(matches!(failing, Err(CliError::JobFailed(e)) if e.to_string() == "boom"));
        assert!(matches!(reporting, Err(CliError::JobFailed(e)) if e.to_string() == "bad output"));
    }

    #[test]
    fn a_panicking_job_fails() {
        let panicking = run(&cli(FakeJob::new("train", Outcome::Panic)), &["train"]);

        assert!(matches!(
            panicking,
            Err(CliError::JobFailed(e)) if e.to_string().contains("kernel exploded")
        ));
    }

    #[test]
    fn a_job_asked_to_stop_is_stopped_whether_it_fails_or_succeeds() {
        for outcome in [Outcome::Cancel, Outcome::Stop] {
            let stopped = run(&cli(FakeJob::new("train", outcome)), &["train"]);

            assert!(matches!(stopped, Err(CliError::Stopped)));
        }
    }

    #[test]
    fn each_outcome_has_its_exit_code() {
        let cli = cli(FakeJob::with_example("train", json!({"epochs": 10})));
        let exit_code = |outcome: Result<(), CliError>| outcome.unwrap_err().exit_code();

        assert_eq!(exit_code(run(&cli, &["infer"])), 2);
        assert_eq!(exit_code(run(&cli, &[])), 2);
        assert_eq!(exit_code(run(&cli, &["train", "{"])), 2);
        assert_eq!(exit_code(run(&cli, &["train", "--epochs", "ten"])), 2);
        assert_eq!(exit_code(run(&cli, &["train", "--epochs"])), 2);
        assert_eq!(exit_code(run(&cli, &["--completions", "tcsh"])), 2);
        assert_eq!(exit_code(run(&cli, &["train", "[1, 2]"])), 2);
        for (outcome, code) in [
            (Outcome::Fail, 1),
            (Outcome::ReportError, 1),
            (Outcome::Panic, 1),
            (Outcome::Cancel, 130),
            (Outcome::Stop, 130),
        ] {
            let cli = self::cli(FakeJob::new("train", outcome));
            assert_eq!(exit_code(run(&cli, &["train"])), code);
        }
    }

    fn read_json(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    /// The fields of the report at `path` that say how the job is going and how it ended.
    fn status(path: &Path) -> (Value, Value, Value) {
        let report = read_json(path);
        (
            report["status"].clone(),
            report["error"].clone(),
            report["experiment"].clone(),
        )
    }

    #[test]
    fn an_inference_is_reported_running_then_completed_with_no_experiment() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");
        let running = Arc::new(Mutex::new(None));
        let words = InferenceModule::new(Arc::new(NoopInferenceProvider::new())).create("words", {
            let (path, running) = (path.clone(), running.clone());
            move |_session: &InferenceSession,
                  input: InferenceInput<String>,
                  output: InferenceOutput<String>| {
                *running.lock().unwrap() = Some(read_json(&path));
                for text in input {
                    let _ = output.write(text);
                }
            }
        });
        let cli = Cli::new().register(words, JsonMapper::<String>::new());

        run_reporting(&cli, &["words", r#""hello""#], Some(&path)).unwrap();

        let running = running.lock().unwrap().take().unwrap();
        assert_eq!(running["job"], "words");
        assert_eq!(running["status"], "running");
        assert_eq!(running["experiment"], Value::Null);
        assert_eq!(
            status(&path),
            (json!("completed"), Value::Null, Value::Null)
        );
        assert!(read_json(&path)["finished_at"].is_string());
    }

    #[test]
    fn an_experiment_is_linked_once_created_and_reported_by_how_its_job_ended() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");
        let runs = dir.path().join("runs");
        let linked = Arc::new(Mutex::new(Vec::new()));
        let train = Experiments::new(Arc::new(LocalExperiments::new(&runs))).create("train", {
            let (path, linked) = (path.clone(), linked.clone());
            move |_run: &ExperimentRun, fail: bool| -> Result<(), BoxError> {
                linked.lock().unwrap().push(read_json(&path));
                if fail {
                    Err("loss is NaN".into())
                } else {
                    Ok(())
                }
            }
        });
        let cli = Cli::new().register(train, JsonMapper::<bool>::new());

        run_reporting(&cli, &["train", "false"], Some(&path)).unwrap();
        let completed = status(&path);
        let failed = run_reporting(&cli, &["train", "true"], Some(&path));

        let runs = runs.canonicalize().unwrap();
        let linked = linked.lock().unwrap();
        for (num, report) in [(1, &linked[0]), (2, &linked[1])] {
            assert_eq!(report["status"], "running");
            assert_eq!(
                report["experiment"],
                json!({"num": num, "url": null, "dir": runs.join(format!("train/{num}"))})
            );
        }
        assert_eq!(
            completed,
            (
                json!("completed"),
                Value::Null,
                linked[0]["experiment"].clone()
            )
        );
        assert!(matches!(failed, Err(CliError::JobFailed(_))));
        assert_eq!(
            status(&path),
            (
                json!("failed"),
                json!("loss is NaN"),
                linked[1]["experiment"].clone()
            )
        );
    }

    #[test]
    fn a_job_asked_to_stop_is_reported_by_what_it_returns() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");
        let report = |outcome| {
            let stopped = run_reporting(
                &cli(FakeJob::new("train", outcome)),
                &["train"],
                Some(&path),
            );
            assert!(matches!(stopped, Err(CliError::Stopped)));
            status(&path)
        };

        assert_eq!(
            report(Outcome::Stop),
            (json!("completed"), Value::Null, Value::Null)
        );
        assert_eq!(
            report(Outcome::Cancel),
            (json!("failed"), json!("interrupted"), Value::Null)
        );
    }

    #[test]
    fn a_failing_or_panicking_job_is_reported_failed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");
        let report = |outcome| {
            let failed = run_reporting(
                &cli(FakeJob::new("train", outcome)),
                &["train"],
                Some(&path),
            );
            assert!(matches!(failed, Err(CliError::JobFailed(_))));
            status(&path)
        };

        assert_eq!(
            report(Outcome::Fail),
            (json!("failed"), json!("boom"), Value::Null)
        );
        assert_eq!(
            report(Outcome::ReportError),
            (json!("failed"), json!("bad output"), Value::Null)
        );
        assert_eq!(
            report(Outcome::Panic),
            (
                json!("failed"),
                json!("the job panicked: kernel exploded"),
                Value::Null
            )
        );
    }

    #[test]
    fn a_rejected_job_or_input_writes_no_report() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");
        let cli = cli(FakeJob::with_example("train", json!({"epochs": 10})));

        for args in [
            &["infer"][..],
            &["train", "[1, 2]"],
            &["train", "--epochs", "ten"],
        ] {
            let rejected = run_reporting(&cli, args, Some(&path));

            assert_eq!(rejected.unwrap_err().exit_code(), 2, "{args:?}");
            assert!(!path.exists(), "{args:?}");
        }
    }

    #[test]
    fn a_report_that_cannot_be_written_fails_the_job_before_it_runs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing").join("report.json");
        let job = FakeJob::new("train", Outcome::Succeed);
        let ran_with = job.ran_with.clone();

        let failed = run_reporting(&cli(job), &["train"], Some(&path));

        let failed = failed.unwrap_err();
        assert!(matches!(failed, CliError::Report(_)), "{failed:?}");
        assert_eq!(failed.exit_code(), 1);
        assert!(ran_with.lock().unwrap().is_none());
    }

    /// The input the job's command line `<job> <args>` gives.
    fn job_command_input(definition: &JobDefinition, args: &[&str]) -> Value {
        let matches = job_command(definition)
            .try_get_matches_from([definition.name.as_str()].iter().chain(args))
            .unwrap();
        job_input(&matches)
    }

    #[test]
    fn the_mapper_merges_the_command_line_input_onto_its_default() {
        let mapper = JsonMapper::with_default(json!({
            "num_epochs": 10,
            "batch_size": 64,
            "shuffle": true,
            "tag": "baseline",
            "layers": [64, 32],
            "resume_from": null,
            "optimizer": {"lr": 0.001, "weight_decay": 5e-5f32}
        }));
        let job = definition("mnist", None, mapper.example());

        let input = job_command_input(
            &job,
            &[
                "--num-epochs",
                "3",
                r#"{"batch_size": 16, "tag": null, "optimizer": {"lr": 0.2, "weight_decay": 0.5}}"#,
                "--optimizer.lr",
                "0.3",
            ],
        );

        // `null` removes the field from the default.
        assert_eq!(
            mapper.resolve(input),
            json!({
                "num_epochs": 3,
                "batch_size": 16,
                "shuffle": true,
                "layers": [64, 32],
                "resume_from": null,
                "optimizer": {"lr": 0.3, "weight_decay": 0.5}
            })
        );
    }

    #[cfg(feature = "schema")]
    #[test]
    fn a_documented_input_type_gives_documented_flags() {
        use serde::{Deserialize, Serialize};

        /// How to train.
        #[derive(Serialize, Deserialize, schemars::JsonSchema)]
        struct Config {
            /// Passes over the data.
            num_epochs: u32,
            optimizer: Optimizer,
            /// Where to train.
            device: Device,
            resume_from: Option<Resume>,
        }

        #[derive(Serialize, Deserialize, schemars::JsonSchema)]
        struct Optimizer {
            /// The learning rate.
            lr: f64,
        }

        #[derive(Serialize, Deserialize, schemars::JsonSchema)]
        #[serde(rename_all = "snake_case")]
        enum Device {
            /// The CPU.
            Cpu,
            /// The first GPU.
            Gpu,
        }

        #[derive(Serialize, Deserialize, schemars::JsonSchema)]
        struct Resume {
            experiment: u32,
        }

        let mapper = JsonMapper::with_default(Config {
            num_epochs: 10,
            optimizer: Optimizer { lr: 0.001 },
            device: Device::Cpu,
            resume_from: None,
        })
        .with_schema();
        let job = definition("train", mapper.schema(), mapper.example());

        let help = job_command(&job).render_long_help().to_string();
        for line in [
            "--device <VALUE>",
            "Where to train.",
            "cpu: The CPU.",
            "--num-epochs <INT>",
            "Passes over the data.",
            "--optimizer.lr <FLOAT>",
            "The learning rate.",
            "--resume-from <JSON>",
        ] {
            assert!(help.contains(line), "no {line} in\n{help}");
        }
        let input = job_command_input(
            &job,
            &["--device", "gpu", "--resume-from", r#"{"experiment": 4}"#],
        );
        let config = mapper.map(input).unwrap();
        assert!(matches!(config.device, Device::Gpu));
        assert_eq!(config.resume_from.map(|resume| resume.experiment), Some(4));
        assert_eq!(config.num_epochs, 10);
    }

    #[test]
    #[should_panic(expected = "already registered")]
    fn a_job_name_registers_once() {
        cli(FakeJob::new("evaluate", Outcome::Succeed));
    }
}
