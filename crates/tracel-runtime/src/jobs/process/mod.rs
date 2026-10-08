//! The process adapter: the program runs as a process, launched with the job and its input on
//! its command line, and answers through its exit code, its run report and the definitions file.

mod describe;
mod error;
mod file;
mod report;
mod signal;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use clap::Command;
use clap::error::{ContextKind, ContextValue, ErrorKind};
use clap_complete::Shell;
use serde_json::Value;
use tracel_job_proto::{TRACEL_JOB_NUM, command, completions, job_input};

use crate::jobs::{Jobs, Launch, PrepareError};
use crate::stop::Stop;
use describe::describe;
use error::RunError;
use report::ReportFile;

/// What a command line asks for.
enum Request {
    /// Run the job `job` with `input`.
    Run { job: String, input: Value },
    /// Print the completion script for `shell`.
    Completions(Shell),
    /// Print the help or the version, which clap gives as an error that is not one.
    Print(clap::Error),
}

/// What the job the process runs is launched with: the request that it stop, and, through the
/// environment, where its run report goes and the number its launcher gave it.
struct ProcessLaunch {
    stop: Stop,
    report_path: Option<PathBuf>,
    num: Option<String>,
}

impl ProcessLaunch {
    /// The launch of a job asked to stop once `stop` is requested, as the environment variables
    /// `lookup` gives describe it.
    fn from_vars(stop: Stop, lookup: impl Fn(&str) -> Option<OsString>) -> Self {
        Self {
            report_path: report::path_from_vars(&lookup),
            num: lookup(TRACEL_JOB_NUM)
                .filter(|num| !num.is_empty())
                .map(|num| num.to_string_lossy().into_owned()),
            stop,
        }
    }
}

impl Jobs {
    /// Runs the job the command-line arguments name, and returns the exit code for the process.
    ///
    /// | Exit code | Meaning |
    /// | --- | --- |
    /// | 0 | The job completed, or the help, version or completion script was printed |
    /// | 1 | The job failed, or a file its launcher asked for could not be written |
    /// | 2 | No job or an unknown job is named, a flag or the input is unusable, or the input does not decode |
    /// | 130 | The job was asked to stop |
    ///
    /// Why it did not complete is printed to stderr, with the registered job names when no job or
    /// an unknown one is named, and a stop request as a warning. Launched by `tracel run`, the
    /// program also gives its launcher its jobs' definitions, how the job is going and the
    /// experiments code in the job reports through [`report_experiment`](crate::report_experiment).
    ///
    /// The job runs on this thread, in its [`JobScope`](crate::JobScope):
    /// [`stop_requested`](crate::stop_requested) says whether it was asked to stop,
    /// [`on_stop`](crate::on_stop) runs code once it is, [`job_num`](crate::job_num) gives the
    /// number its launcher gave it, and [`job_input`](crate::job_input) the input it runs with.
    ///
    /// SIGTERM, SIGINT or SIGHUP, or Ctrl-C, Ctrl-Break or closing the console on Windows, asks
    /// the job to stop: it runs what [`on_stop`](crate::on_stop) registered, the job ends as
    /// completed or failed by what it returns, and the process exits with code 130. A second
    /// signal ends the process at once, also with exit code 130. `tracel run` sends SIGKILL after
    /// a 30-second grace period.
    pub fn run(self) -> ExitCode {
        match self.execute() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                error.print();
                ExitCode::from(error.exit_code())
            }
        }
    }

    fn execute(self) -> Result<(), RunError> {
        let vars = |name: &str| std::env::var_os(name);
        if describe(self.definitions(), vars)? {
            return Ok(());
        }
        let mut command = self.command_line(program_name());
        match self.parse(&mut command, std::env::args_os())? {
            Request::Run { job, input } => {
                let stop = Stop::default();
                signal::stop_on_termination(stop.clone());
                self.dispatch(&job, input, ProcessLaunch::from_vars(stop, vars))
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
        let command = command(name, self.definitions());
        match &self.version {
            Some(version) => command.version(version.clone()),
            None => command,
        }
    }

    /// Reads what the command-line arguments `args`, the program's name first, ask for.
    fn parse<I, T>(&self, command: &mut Command, args: I) -> Result<Request, RunError>
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
                return Err(RunError::UnknownJob {
                    name,
                    available: self.names(),
                });
            }
            Err(error) => return Err(RunError::Usage(error)),
        };
        if let Some(shell) = completions(&matches) {
            return Ok(Request::Completions(shell));
        }
        match (matches.subcommand(), &self.default) {
            (Some((job, matches)), _) => Ok(Request::Run {
                job: job.to_string(),
                input: job_input(matches, |path| std::fs::read_to_string(path))
                    .map_err(RunError::Config)?,
            }),
            (None, Some(job)) => Ok(Request::Run {
                job: job.clone(),
                input: Value::Null,
            }),
            (None, None) => Err(RunError::MissingJob {
                available: self.names(),
            }),
        }
    }

    /// Runs the job `name` with `input`, as `launch` launches it.
    fn dispatch(&self, name: &str, input: Value, launch: ProcessLaunch) -> Result<(), RunError> {
        let prepared = self.prepare(name, input).map_err(|error| match error {
            PrepareError::UnknownJob => RunError::UnknownJob {
                name: name.to_string(),
                available: self.names(),
            },
            PrepareError::InvalidInput(error) => RunError::InvalidInput(error),
        })?;
        let report = launch
            .report_path
            .map(|path| ReportFile::start(path, name))
            .transpose()?
            .map(Arc::new);

        let experiments = report.clone();
        let outcome = crate::jobs::run(
            prepared,
            Launch {
                stop: launch.stop.clone(),
                num: launch.num,
                experiments: Arc::new(move |experiment| {
                    if let Some(report) = &experiments
                        && let Err(error) = report.add_experiment(experiment)
                    {
                        eprintln!("warning: {error}");
                    }
                }),
            },
        );

        if let Some(report) = report
            && let Err(error) = report.finish(&outcome)
        {
            eprintln!("warning: {error}");
        }
        if launch.stop.is_requested() {
            return Err(RunError::Stopped);
        }
        outcome.map_err(RunError::JobFailed)
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

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::sync::Mutex;

    use serde::{Deserialize, Serialize};
    use serde_json::json;
    use tracel_job_proto::{ReportedExperiment, TRACEL_REPORT_FILE};

    use super::*;
    use crate::scope::request_stop;
    use crate::{
        BoxError, IntoJob, Job, job_input, job_num, job_running, on_stop, report_experiment,
        stop_requested,
    };

    #[derive(Clone, Copy)]
    enum Outcome {
        Succeed,
        Fail,
        Panic,
        /// Asks itself to stop, then fails.
        Cancel,
        /// Asks itself to stop, then succeeds.
        Stop,
    }

    /// The input a job ran with, once it ran.
    type RanWith = Arc<Mutex<Option<Value>>>;

    /// A job that takes any JSON input, keeps it in `ran_with`, then ends with `outcome`.
    fn job(
        outcome: Outcome,
        ran_with: &RanWith,
    ) -> impl Fn(Value) -> Result<(), BoxError> + Send + Sync + use<> {
        let ran_with = ran_with.clone();
        move |input| {
            *ran_with.lock().unwrap() = Some(input);
            match outcome {
                Outcome::Succeed => Ok(()),
                Outcome::Fail => Err("boom".into()),
                Outcome::Panic => panic!("kernel exploded"),
                Outcome::Cancel => {
                    request_stop();
                    Err("interrupted".into())
                }
                Outcome::Stop => {
                    request_stop();
                    Ok(())
                }
            }
        }
    }

    /// `train`, which runs `train`, and `evaluate`, which succeeds.
    fn jobs(train: impl IntoJob<Value>) -> Jobs {
        Jobs::new()
            .job("train", train)
            .job("evaluate", job(Outcome::Succeed, &RanWith::default()))
    }

    /// `train`, which ends with `outcome`, and `evaluate`, and the input `train` ran with.
    fn ending(outcome: Outcome) -> (Jobs, RanWith) {
        let ran_with = RanWith::default();
        (jobs(job(outcome, &ran_with)), ran_with)
    }

    /// `train`, which succeeds with its input merged onto `default`, and `evaluate`, and the input
    /// `train` ran with.
    fn with_default(default: Value) -> (Jobs, RanWith) {
        let ran_with = RanWith::default();
        let train = Job::new(job(Outcome::Succeed, &ran_with)).with_default(default);
        (jobs(train), ran_with)
    }

    #[derive(Serialize, Deserialize)]
    struct Epochs {
        epochs: u32,
    }

    /// `train`, which takes `Epochs`, ten by default, and `evaluate`, and whether `train` ran.
    fn typed() -> (Jobs, Arc<Mutex<bool>>) {
        let ran = Arc::new(Mutex::new(false));
        let train = Job::new({
            let ran = ran.clone();
            move |_: Epochs| {
                *ran.lock().unwrap() = true;
                Ok::<_, BoxError>(())
            }
        })
        .with_default(Epochs { epochs: 10 });
        let jobs = Jobs::new()
            .job("train", train)
            .job("evaluate", job(Outcome::Succeed, &RanWith::default()));
        (jobs, ran)
    }

    /// What the command line `program <args>` asks of `jobs`.
    fn parse(jobs: &Jobs, args: &[&str]) -> Result<Request, RunError> {
        let mut command = jobs.command_line("program");
        jobs.parse(&mut command, ["program"].iter().chain(args))
    }

    /// Runs the command line `program <args>` as [`Jobs::run`] does, printing nothing.
    fn run(jobs: Jobs, args: &[&str]) -> Result<(), RunError> {
        run_launched(jobs, args, &[])
    }

    /// Runs the command line `program <args>` as [`run`] does, launched with the environment
    /// variables `vars`.
    fn run_launched(jobs: Jobs, args: &[&str], vars: &[(&str, &OsStr)]) -> Result<(), RunError> {
        match parse(&jobs, args)? {
            Request::Run { job, input } => {
                let launch = ProcessLaunch::from_vars(Stop::default(), |name| {
                    vars.iter()
                        .find(|(variable, _)| *variable == name)
                        .map(|(_, value)| value.to_os_string())
                });
                jobs.dispatch(&job, input, launch)
            }
            Request::Completions(_) | Request::Print(_) => Ok(()),
        }
    }

    /// Runs the command line `program <args>` as [`run`] does, keeping the run report at `path`.
    fn run_reporting(jobs: Jobs, args: &[&str], path: &Path) -> Result<(), RunError> {
        run_launched(jobs, args, &[(TRACEL_REPORT_FILE, path.as_os_str())])
    }

    #[test]
    fn the_named_job_runs_with_its_json_input() {
        let (jobs, ran_with) = ending(Outcome::Succeed);

        run(jobs, &["train", r#"{"epochs": 2, "tag": null}"#]).unwrap();

        assert_eq!(
            *ran_with.lock().unwrap(),
            Some(json!({"epochs": 2, "tag": null}))
        );
    }

    #[test]
    fn a_missing_input_runs_the_job_with_its_default_or_no_input() {
        let (jobs, ran_with) = ending(Outcome::Succeed);
        let (defaulted, ran_with_default) = with_default(json!({"epochs": 10}));
        let ran = Arc::new(Mutex::new(false));
        let unit = Jobs::new().job("evaluate", {
            let ran = ran.clone();
            move |()| {
                *ran.lock().unwrap() = true;
                Ok::<_, BoxError>(())
            }
        });

        run(jobs, &["train"]).unwrap();
        run(defaulted, &["train"]).unwrap();
        run(unit, &["evaluate"]).unwrap();

        assert_eq!(*ran_with.lock().unwrap(), Some(Value::Null));
        assert_eq!(
            *ran_with_default.lock().unwrap(),
            Some(json!({"epochs": 10}))
        );
        assert!(*ran.lock().unwrap());
    }

    #[test]
    fn flags_set_the_fields_of_the_input() {
        let (jobs, ran_with) =
            with_default(json!({"epochs": 10, "optimizer": {"weight_decay": 0.0}}));

        run(
            jobs,
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
    fn the_command_line_input_is_merged_onto_the_default() {
        let (jobs, ran_with) = with_default(json!({
            "num_epochs": 10,
            "batch_size": 64,
            "shuffle": true,
            "tag": "baseline",
            "layers": [64, 32],
            "resume_from": null,
            "optimizer": {"lr": 0.001, "weight_decay": 5e-5f32}
        }));

        run(
            jobs,
            &[
                "train",
                "--num-epochs",
                "3",
                r#"{"batch_size": 16, "tag": null, "optimizer": {"lr": 0.2, "weight_decay": 0.5}}"#,
                "--optimizer.lr",
                "0.3",
            ],
        )
        .unwrap();

        // `null` removes the field from the default.
        assert_eq!(
            *ran_with.lock().unwrap(),
            Some(json!({
                "num_epochs": 3,
                "batch_size": 16,
                "shuffle": true,
                "layers": [64, 32],
                "resume_from": null,
                "optimizer": {"lr": 0.3, "weight_decay": 0.5}
            }))
        );
    }

    #[test]
    fn the_config_file_is_read_and_an_unreadable_one_rejects_the_input() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.json");
        std::fs::write(&config, r#"{"epochs": 3}"#).unwrap();
        let missing = dir.path().join("missing.json");
        let (jobs, ran_with) = with_default(json!({"epochs": 10, "lr": 0.1}));
        let (rejected_jobs, rejected) = with_default(json!({"epochs": 10}));

        run(jobs, &["train", "-c", config.to_str().unwrap()]).unwrap();
        let unreadable = run(rejected_jobs, &["train", "-c", missing.to_str().unwrap()]);

        assert_eq!(
            *ran_with.lock().unwrap(),
            Some(json!({"epochs": 3, "lr": 0.1}))
        );
        let unreadable = unreadable.unwrap_err();
        assert!(matches!(unreadable, RunError::Config(_)), "{unreadable:?}");
        assert_eq!(unreadable.exit_code(), 2);
        assert!(rejected.lock().unwrap().is_none());
    }

    #[test]
    fn the_default_job_runs_when_no_job_is_named() {
        let (jobs, ran_with) = ending(Outcome::Succeed);

        run(jobs.default_job("train"), &[]).unwrap();

        assert_eq!(*ran_with.lock().unwrap(), Some(Value::Null));
    }

    #[test]
    fn an_unknown_or_missing_job_lists_the_registered_ones() {
        let jobs = || ending(Outcome::Succeed).0;

        let unknown = run(jobs(), &["infer"]).unwrap_err();
        let missing = run(jobs(), &[]).unwrap_err();
        let unknown_default = run(jobs().default_job("infer"), &[]).unwrap_err();

        for unknown in [&unknown, &unknown_default] {
            assert!(
                matches!(
                    unknown,
                    RunError::UnknownJob { name, available }
                        if name == "infer" && available == &["evaluate", "train"]
                ),
                "{unknown:?}"
            );
            assert!(unknown.to_string().contains("evaluate, train"), "{unknown}");
        }
        assert!(matches!(missing, RunError::MissingJob { .. }));
        assert!(missing.to_string().contains("evaluate, train"), "{missing}");
    }

    #[test]
    fn an_unusable_command_line_or_an_input_that_does_not_decode_is_invalid() {
        let rejected = |args: &[&str]| {
            let (jobs, ran) = typed();
            let rejected = run(jobs, args);
            assert!(!*ran.lock().unwrap(), "{args:?}");
            rejected
        };

        let not_json = rejected(&["train", "{"]);
        let not_an_integer = rejected(&["train", "--epochs", "ten"]);
        let unknown_flag = rejected(&["train", "--epoch", "2"]);
        let not_decoded = rejected(&["train", "[1, 2]"]);
        let wrong_type = rejected(&["train", r#"{"epochs": "ten"}"#]);

        for usage in [not_json, not_an_integer, unknown_flag] {
            assert!(matches!(usage, Err(RunError::Usage(_))), "{usage:?}");
        }
        for invalid in [not_decoded, wrong_type] {
            assert!(
                matches!(invalid, Err(RunError::InvalidInput(_))),
                "{invalid:?}"
            );
        }
    }

    #[test]
    fn help_and_completions_are_printed_instead_of_running_a_job() {
        let (jobs, _) = with_default(json!({"epochs": 10}));

        for args in [&["--help"][..], &["train", "--help"], &["train", "-h"]] {
            assert!(
                matches!(parse(&jobs, args), Ok(Request::Print(help)) if help.exit_code() == 0),
                "{args:?}"
            );
        }
        assert!(matches!(
            parse(&jobs, &["--completions", "bash"]),
            Ok(Request::Completions(Shell::Bash))
        ));
        assert!(matches!(
            parse(&jobs, &["--completions", "bash", "train"]),
            Err(RunError::Usage(_))
        ));
    }

    #[test]
    fn the_version_the_program_gives_is_printed_with_its_name() {
        let jobs = ending(Outcome::Succeed).0.version("1.2.3");

        let Ok(Request::Print(version)) = parse(&jobs, &["--version"]) else {
            panic!("--version printed nothing");
        };

        assert_eq!(version.kind(), ErrorKind::DisplayVersion);
        assert!(!version.use_stderr());
        assert_eq!(version.exit_code(), 0);
        assert_eq!(version.to_string(), "program 1.2.3\n");
    }

    #[test]
    fn without_a_version_there_is_no_version_flag() {
        let jobs = ending(Outcome::Succeed).0;

        let unoffered = parse(&jobs, &["--version"]);

        assert!(
            matches!(
                &unoffered,
                Err(RunError::Usage(error)) if error.kind() == ErrorKind::UnknownArgument
            ),
            "{:?}",
            unoffered.as_ref().err()
        );
        assert_eq!(unoffered.err().map(|error| error.exit_code()), Some(2));
    }

    #[test]
    fn a_failing_or_panicking_job_fails() {
        let failing = run(ending(Outcome::Fail).0, &["train"]);
        let panicking = run(ending(Outcome::Panic).0, &["train"]);

        assert!(matches!(failing, Err(RunError::JobFailed(e)) if e.to_string() == "boom"));
        assert!(matches!(
            panicking,
            Err(RunError::JobFailed(e)) if e.to_string().contains("kernel exploded")
        ));
    }

    #[test]
    fn a_job_asked_to_stop_is_stopped_whether_it_fails_or_succeeds() {
        for outcome in [Outcome::Cancel, Outcome::Stop] {
            let stopped = run(ending(outcome).0, &["train"]);

            assert!(matches!(stopped, Err(RunError::Stopped)));
        }
    }

    #[test]
    fn each_outcome_has_its_exit_code() {
        let exit_code = |args: &[&str]| run(typed().0, args).unwrap_err().exit_code();

        assert_eq!(exit_code(&["infer"]), 2);
        assert_eq!(exit_code(&[]), 2);
        assert_eq!(exit_code(&["train", "{"]), 2);
        assert_eq!(exit_code(&["train", "--epochs", "ten"]), 2);
        assert_eq!(exit_code(&["train", "--epochs"]), 2);
        assert_eq!(exit_code(&["--completions", "tcsh"]), 2);
        assert_eq!(exit_code(&["train", "[1, 2]"]), 2);
        for (outcome, code) in [
            (Outcome::Fail, 1),
            (Outcome::Panic, 1),
            (Outcome::Cancel, 130),
            (Outcome::Stop, 130),
        ] {
            let failed = run(ending(outcome).0, &["train"]);
            assert_eq!(failed.unwrap_err().exit_code(), code);
        }
    }

    #[test]
    fn the_job_is_the_running_job_until_it_ends() {
        let given = Arc::new(Mutex::new(Vec::new()));
        let numbered = |num: &str| {
            let jobs = Jobs::new().job("train", {
                let given = given.clone();
                move |()| {
                    given
                        .lock()
                        .unwrap()
                        .push((job_running(), stop_requested(), job_num()));
                    Ok::<_, BoxError>(())
                }
            });
            run_launched(jobs, &["train"], &[(TRACEL_JOB_NUM, OsStr::new(num))]).unwrap();
        };

        numbered("12");
        numbered("");
        let after = (job_running(), job_num());

        assert_eq!(
            *given.lock().unwrap(),
            [(true, false, Some("12".to_string())), (true, false, None)]
        );
        assert_eq!(after, (false, None));
    }

    #[test]
    fn what_runs_on_stop_runs_when_the_job_is_asked_to_stop() {
        let ran = Arc::new(Mutex::new(Vec::new()));
        let jobs = Jobs::new().job("train", {
            let ran = ran.clone();
            move |()| {
                let on_stop_ran = ran.clone();
                on_stop(move || on_stop_ran.lock().unwrap().push("on stop"));
                ran.lock().unwrap().push("before");
                request_stop();
                ran.lock().unwrap().push("after");
                Ok::<_, BoxError>(())
            }
        });

        let stopped = run(jobs, &["train"]);

        assert!(matches!(stopped, Err(RunError::Stopped)));
        assert_eq!(*ran.lock().unwrap(), ["before", "on stop", "after"]);
    }

    #[test]
    fn the_running_job_is_given_the_input_it_runs_with() {
        let given = Arc::new(Mutex::new(Vec::new()));
        let train = Job::new({
            let given = given.clone();
            move |input: Value| {
                given.lock().unwrap().push((job_input(), Some(input)));
                Ok::<_, BoxError>(())
            }
        })
        .with_default(json!({"epochs": 10, "optimizer": {"lr": 0.001}}));
        let evaluate = {
            let given = given.clone();
            move |()| {
                given.lock().unwrap().push((job_input(), None));
                Ok::<_, BoxError>(())
            }
        };

        run(Jobs::new().job("train", train), &["train", "--epochs", "2"]).unwrap();
        run(Jobs::new().job("evaluate", evaluate), &["evaluate"]).unwrap();

        let merged = json!({"epochs": 2, "optimizer": {"lr": 0.001}});
        assert_eq!(
            *given.lock().unwrap(),
            [
                (Some(merged.clone()), Some(merged)),
                (Some(Value::Null), None)
            ]
        );
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
            report["experiments"].clone(),
        )
    }

    fn offline(num: u64) -> ReportedExperiment {
        ReportedExperiment {
            num: Some(num),
            url: None,
            dir: Some(format!("runs/train/{num}").into()),
        }
    }

    #[test]
    fn a_job_that_starts_no_experiment_is_reported_running_then_completed_with_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");
        let running = Arc::new(Mutex::new(None));
        let jobs = Jobs::new().job("evaluate", {
            let (path, running) = (path.clone(), running.clone());
            move |()| {
                *running.lock().unwrap() = Some(read_json(&path));
                Ok::<_, BoxError>(())
            }
        });

        run_reporting(jobs, &["evaluate"], &path).unwrap();

        let running = running.lock().unwrap().take().unwrap();
        assert_eq!(running["job"], "evaluate");
        assert_eq!(running["status"], "running");
        assert_eq!(running["experiments"], json!([]));
        assert_eq!(status(&path), (json!("completed"), Value::Null, json!([])));
        assert!(read_json(&path)["finished_at"].is_string());
    }

    #[test]
    fn each_experiment_the_job_reports_is_listed_as_it_starts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");
        let listed = Arc::new(Mutex::new(Vec::new()));
        let jobs = |fail: bool| {
            Jobs::new().job("train", {
                let (path, listed) = (path.clone(), listed.clone());
                move |()| -> Result<(), BoxError> {
                    for num in [1, 2] {
                        report_experiment(offline(num));
                        listed.lock().unwrap().push(read_json(&path));
                    }
                    if fail {
                        Err("loss is NaN".into())
                    } else {
                        Ok(())
                    }
                }
            })
        };

        run_reporting(jobs(false), &["train"], &path).unwrap();
        let completed = status(&path);
        let failed = run_reporting(jobs(true), &["train"], &path);

        let both = json!([
            {"num": 1, "url": null, "dir": "runs/train/1"},
            {"num": 2, "url": null, "dir": "runs/train/2"}
        ]);
        let listed = listed.lock().unwrap();
        assert_eq!(listed[0]["status"], "running");
        assert_eq!(listed[0]["experiments"], json!([both[0]]));
        assert_eq!(listed[1]["experiments"], both);
        assert_eq!(completed, (json!("completed"), Value::Null, both.clone()));
        assert!(matches!(failed, Err(RunError::JobFailed(_))));
        assert_eq!(status(&path), (json!("failed"), json!("loss is NaN"), both));
    }

    #[test]
    fn a_job_asked_to_stop_is_reported_by_what_it_returns() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");
        let report = |outcome| {
            let stopped = run_reporting(ending(outcome).0, &["train"], &path);
            assert!(matches!(stopped, Err(RunError::Stopped)));
            status(&path)
        };

        assert_eq!(
            report(Outcome::Stop),
            (json!("completed"), Value::Null, json!([]))
        );
        assert_eq!(
            report(Outcome::Cancel),
            (json!("failed"), json!("interrupted"), json!([]))
        );
    }

    #[test]
    fn a_failing_or_panicking_job_is_reported_failed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");
        let report = |outcome| {
            let failed = run_reporting(ending(outcome).0, &["train"], &path);
            assert!(matches!(failed, Err(RunError::JobFailed(_))));
            status(&path)
        };

        assert_eq!(
            report(Outcome::Fail),
            (json!("failed"), json!("boom"), json!([]))
        );
        assert_eq!(
            report(Outcome::Panic),
            (
                json!("failed"),
                json!("the job panicked: kernel exploded"),
                json!([])
            )
        );
    }

    #[test]
    fn a_rejected_job_or_input_writes_no_report() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");

        for args in [
            &["infer"][..],
            &["train", "[1, 2]"],
            &["train", "--epochs", "ten"],
        ] {
            let rejected = run_reporting(typed().0, args, &path);

            assert_eq!(rejected.unwrap_err().exit_code(), 2, "{args:?}");
            assert!(!path.exists(), "{args:?}");
        }
    }

    #[test]
    fn a_report_that_cannot_be_written_fails_the_job_before_it_runs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing").join("report.json");
        let (jobs, ran_with) = ending(Outcome::Succeed);

        let failed = run_reporting(jobs, &["train"], &path);

        let failed = failed.unwrap_err();
        assert!(matches!(failed, RunError::Report(_)), "{failed:?}");
        assert_eq!(failed.exit_code(), 1);
        assert!(ran_with.lock().unwrap().is_none());
    }

    #[cfg(feature = "schema")]
    #[test]
    fn a_documented_input_type_gives_documented_flags() {
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

        let ran_with = Arc::new(Mutex::new(None));
        let train = Job::new({
            let ran_with = ran_with.clone();
            move |config: Config| {
                *ran_with.lock().unwrap() = Some(config);
                Ok::<_, BoxError>(())
            }
        })
        .with_default(Config {
            num_epochs: 10,
            optimizer: Optimizer { lr: 0.001 },
            device: Device::Cpu,
            resume_from: None,
        })
        .with_schema();
        let jobs = Jobs::new().job("train", train);

        let help = tracel_job_proto::job_command(jobs.jobs["train"].definition())
            .render_long_help()
            .to_string();
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
        run(
            jobs,
            &[
                "train",
                "--device",
                "gpu",
                "--resume-from",
                r#"{"experiment": 4}"#,
            ],
        )
        .unwrap();
        let config = ran_with.lock().unwrap().take().unwrap();
        assert!(matches!(config.device, Device::Gpu));
        assert_eq!(config.resume_from.map(|resume| resume.experiment), Some(4));
        assert_eq!(config.num_epochs, 10);
    }

    #[test]
    #[should_panic(expected = "already registered")]
    fn a_job_name_registers_once() {
        jobs(job(Outcome::Succeed, &RanWith::default()))
            .job("evaluate", |()| Ok::<_, BoxError>(()));
    }
}
