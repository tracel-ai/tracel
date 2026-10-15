//! The process adapter: the program runs as a process, launched with the job and its input on
//! its command line, and answers through its exit code and the definitions file.

mod describe;
mod error;
mod file;

use std::ffi::OsString;
use std::path::Path;
use std::process::ExitCode;

use clap::Command;
use clap::error::{ContextKind, ContextValue, ErrorKind};
use clap_complete::Shell;
use serde_json::Value;
use tracel_job::TRACEL_JOB_NUM;
use tracel_job::cli::{command, completions, job_input};

use crate::{Execution, Jobs};
use describe::describe;
use error::RunError;

/// What a command line asks for.
enum Request {
    /// Run the job `job` with `input`.
    Run { job: String, input: Value },
    /// Print the completion script for `shell`.
    Completions(Shell),
    /// Print the help or the version, which clap gives as an error that is not one.
    Print(clap::Error),
}

impl Jobs {
    /// Runs the job the command-line arguments name, and returns the exit code for the process.
    ///
    /// | Exit code | Meaning |
    /// | --- | --- |
    /// | 0 | The job completed, or the help, version or completion script was printed |
    /// | 1 | The job failed, or the job definitions its launcher asked for could not be written |
    /// | 2 | No job or an unknown job is named, a flag or the input is unusable, or the input does not decode |
    ///
    /// Why it did not complete is printed to stderr, with the registered job names when no job or
    /// an unknown one is named. Launched by `tracel run`, the program also gives its launcher its
    /// jobs' definitions.
    ///
    /// The job runs on this thread, in its [`JobScope`](crate::JobScope):
    /// [`job_num`](crate::job_num) gives the number its launcher gave it, and
    /// [`job_input`](crate::job_input) the input it runs with.
    pub fn run(self) -> ExitCode {
        let exit = match execute(&self) {
            Ok(()) => tracel_job::ExitCode::Completed,
            Err(error) => {
                error.print();
                error.exit_code()
            }
        };
        ExitCode::from(exit)
    }
}

/// Does what the process's environment and command line ask of `jobs`: writes their definitions,
/// prints the help, the version or a completion script, or runs the job named.
fn execute(jobs: &Jobs) -> Result<(), RunError> {
    let vars = |name: &str| std::env::var_os(name);
    if describe(jobs.definitions(), vars)? {
        return Ok(());
    }
    let mut command = command_line(jobs, program_name());
    match read_request(jobs, &mut command, std::env::args_os())? {
        Request::Run { job, input } => dispatch(jobs, &job, input, execution(vars)),
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

/// The command line of the program `name`: the [`command`] of `jobs`, with `--version` when the
/// program gives its version.
fn command_line(jobs: &Jobs, name: impl Into<String>) -> Command {
    let command = command(name, jobs.definitions());
    match jobs.get_version() {
        Some(version) => command.version(version.to_string()),
        None => command,
    }
}

/// The names of the jobs of `jobs`, in order.
fn names(jobs: &Jobs) -> Vec<String> {
    jobs.definitions()
        .map(|definition| definition.name.clone())
        .collect()
}

/// Reads what the command-line arguments `args`, the program's name first, ask of `jobs`.
fn read_request<I, T>(jobs: &Jobs, command: &mut Command, args: I) -> Result<Request, RunError>
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
                available: names(jobs),
            });
        }
        Err(error) => return Err(RunError::Usage(error)),
    };
    if let Some(shell) = completions(&matches) {
        return Ok(Request::Completions(shell));
    }
    match (matches.subcommand(), jobs.get_default_job()) {
        (Some((job, matches)), _) => Ok(Request::Run {
            job: job.to_string(),
            input: job_input(matches, |path| std::fs::read_to_string(path))
                .map_err(RunError::Config)?,
        }),
        (None, Some(job)) => Ok(Request::Run {
            job: job.to_string(),
            input: Value::Null,
        }),
        (None, None) => Err(RunError::MissingJob {
            available: names(jobs),
        }),
    }
}

/// The execution of the job the process runs, as the environment variables `lookup` gives
/// describe it: numbered by [`TRACEL_JOB_NUM`] when its launcher sets it.
fn execution(lookup: impl Fn(&str) -> Option<OsString>) -> Execution {
    match lookup(TRACEL_JOB_NUM).filter(|num| !num.is_empty()) {
        Some(num) => Execution::new().with_num(num.to_string_lossy()),
        None => Execution::new(),
    }
}

/// Runs the job `name` of `jobs` with `input`, launched as `execution` says.
fn dispatch(jobs: &Jobs, name: &str, input: Value, execution: Execution) -> Result<(), RunError> {
    jobs.prepare(name, input)?
        .run(execution)
        .into_result()
        .map_err(RunError::JobFailed)
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
    use std::sync::{Arc, Mutex};

    use serde::{Deserialize, Serialize};
    use serde_json::json;

    use super::*;
    use crate::{BoxError, IntoJob, Job, job_input, job_num, job_running};

    #[derive(Clone, Copy)]
    enum Outcome {
        Succeed,
        Fail,
        Panic,
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
        let mut command = command_line(jobs, "program");
        read_request(jobs, &mut command, ["program"].iter().chain(args))
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
                let execution = execution(|name| {
                    vars.iter()
                        .find(|(variable, _)| *variable == name)
                        .map(|(_, value)| value.to_os_string())
                });
                dispatch(&jobs, &job, input, execution)
            }
            Request::Completions(_) | Request::Print(_) => Ok(()),
        }
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
    fn a_job_without_an_input_schema_takes_no_flags() {
        let (jobs, ran_with) = with_default(json!({"epochs": 10}));

        let flag = run(jobs, &["train", "--epochs", "2"]).unwrap_err();

        assert!(
            matches!(&flag, RunError::Usage(error) if error.kind() == ErrorKind::UnknownArgument),
            "{flag:?}"
        );
        assert_eq!(flag.exit_code().code(), 2);
        assert!(ran_with.lock().unwrap().is_none());
    }

    #[test]
    fn the_json_input_is_merged_onto_the_default() {
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
                r#"{"num_epochs": 3, "batch_size": 16, "tag": null, "optimizer": {"lr": 0.3, "weight_decay": 0.5}}"#,
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
        assert_eq!(unreadable.exit_code().code(), 2);
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
        let unknown_flag = rejected(&["train", "--epochs", "2"]);
        let not_decoded = rejected(&["train", "[1, 2]"]);
        let wrong_type = rejected(&["train", r#"{"epochs": "ten"}"#]);

        for usage in [not_json, unknown_flag] {
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
        assert_eq!(
            unoffered.err().map(|error| error.exit_code().code()),
            Some(2)
        );
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
    fn each_outcome_has_its_exit_code() {
        let exit_code = |args: &[&str]| run(typed().0, args).unwrap_err().exit_code().code();

        assert_eq!(exit_code(&["infer"]), 2);
        assert_eq!(exit_code(&[]), 2);
        assert_eq!(exit_code(&["train", "{"]), 2);
        assert_eq!(exit_code(&["train", "--epochs", "ten"]), 2);
        assert_eq!(exit_code(&["train", "--epochs"]), 2);
        assert_eq!(exit_code(&["--completions", "tcsh"]), 2);
        assert_eq!(exit_code(&["train", "[1, 2]"]), 2);
        for (outcome, code) in [(Outcome::Fail, 1), (Outcome::Panic, 1)] {
            let failed = run(ending(outcome).0, &["train"]);
            assert_eq!(failed.unwrap_err().exit_code().code(), code);
        }
    }

    #[test]
    fn the_job_is_the_running_job_until_it_ends() {
        let given = Arc::new(Mutex::new(Vec::new()));
        let numbered = |num: &str| {
            let jobs = Jobs::new().job("train", {
                let given = given.clone();
                move |()| {
                    given.lock().unwrap().push((job_running(), job_num()));
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
            [(true, Some("12".to_string())), (true, None)]
        );
        assert_eq!(after, (false, None));
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

        run(
            Jobs::new().job("train", train),
            &["train", r#"{"epochs": 2}"#],
        )
        .unwrap();
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

        let help = command_line(&jobs, "program")
            .find_subcommand_mut("train")
            .unwrap()
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

    #[cfg(feature = "schema")]
    #[test]
    fn flags_from_the_input_schema_set_fields_after_the_json_input() {
        #[derive(Serialize, Deserialize, schemars::JsonSchema)]
        struct Training {
            epochs: u32,
            optimizer: Optimizer,
        }

        #[derive(Serialize, Deserialize, schemars::JsonSchema)]
        struct Optimizer {
            lr: f64,
            weight_decay: f64,
        }

        let ran_with = RanWith::default();
        let jobs = || {
            let train = Job::new({
                let ran_with = ran_with.clone();
                move |config: Training| {
                    *ran_with.lock().unwrap() = Some(serde_json::to_value(config)?);
                    Ok::<_, BoxError>(())
                }
            })
            .with_default(Training {
                epochs: 10,
                optimizer: Optimizer {
                    lr: 0.001,
                    weight_decay: 0.0,
                },
            })
            .with_schema();
            Jobs::new().job("train", train)
        };

        run(
            jobs(),
            &[
                "train",
                "--optimizer.weight-decay",
                "0.01",
                r#"{"epochs": 2, "optimizer": {"weight_decay": 0.5}}"#,
            ],
        )
        .unwrap();
        let not_an_integer = run(jobs(), &["train", "--epochs", "ten"]).unwrap_err();

        assert_eq!(
            ran_with.lock().unwrap().take(),
            Some(json!({"epochs": 2, "optimizer": {"lr": 0.001, "weight_decay": 0.01}}))
        );
        assert!(
            matches!(not_an_integer, RunError::Usage(_)),
            "{not_an_integer:?}"
        );
        assert!(ran_with.lock().unwrap().is_none());
    }

    #[test]
    #[should_panic(expected = "already registered")]
    fn a_job_name_registers_once() {
        jobs(job(Outcome::Succeed, &RanWith::default()))
            .job("evaluate", |()| Ok::<_, BoxError>(()));
    }
}
