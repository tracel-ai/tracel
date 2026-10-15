//! The process adapter: the program runs as a process, launched with the job and its input on
//! its command line, and answers through its exit code and the definitions file.

mod describe;
mod error;
mod file;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Command;
use clap::error::{ContextKind, ContextValue, ErrorKind};
use clap_complete::Shell;
use serde_json::Value;
use tracel_job::cli::{command, completions, job_input};
use tracel_job::{TRACEL_DESCRIBE, TRACEL_JOB_NUM};

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
        let launch = Launch::new(
            std::env::var_os(TRACEL_DESCRIBE),
            std::env::var_os(TRACEL_JOB_NUM),
        );
        let exit = match execute(&self, std::env::args_os(), launch) {
            Ok(()) => tracel_job::ExitCode::Completed,
            Err(error) => {
                error.print();
                error.exit_code()
            }
        };
        ExitCode::from(exit)
    }
}

/// What the program's launcher asks of it through [`TRACEL_DESCRIBE`] and [`TRACEL_JOB_NUM`].
struct Launch {
    /// Where to write the jobs' definitions, instead of running a job.
    describe: Option<PathBuf>,
    /// The number of the job the program runs.
    num: Option<String>,
}

impl Launch {
    /// The launch the values of `TRACEL_DESCRIBE` and `TRACEL_JOB_NUM` ask for, an empty value
    /// counting as unset.
    fn new(describe: Option<OsString>, num: Option<OsString>) -> Self {
        let set = |value: Option<OsString>| value.filter(|value| !value.is_empty());
        Self {
            describe: set(describe).map(PathBuf::from),
            num: set(num).map(|num| num.to_string_lossy().into_owned()),
        }
    }

    /// The execution of the job the program runs: numbered when its launcher numbered it.
    fn execution(&self) -> Execution {
        match &self.num {
            Some(num) => Execution::new().with_num(num.clone()),
            None => Execution::new(),
        }
    }
}

/// Does what the command-line arguments `args`, the program's name first, and the `launch` ask
/// of `jobs`: writes their definitions, prints the help, the version or a completion script, or
/// runs the job named.
fn execute(
    jobs: &Jobs,
    args: impl IntoIterator<Item = OsString>,
    launch: Launch,
) -> Result<(), RunError> {
    if let Some(path) = &launch.describe {
        return Ok(describe(jobs.definitions(), path)?);
    }
    let args: Vec<OsString> = args.into_iter().collect();
    let mut command = command_line(jobs, program_name(args.first()));
    match read_request(jobs, &mut command, args)? {
        Request::Run { job, input } => dispatch(jobs, &job, input, launch.execution()),
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

/// Runs the job `name` of `jobs` with `input`, launched as `execution` says.
fn dispatch(jobs: &Jobs, name: &str, input: Value, execution: Execution) -> Result<(), RunError> {
    jobs.prepare(name, input)?
        .run(execution)
        .into_result()
        .map_err(RunError::JobFailed)
}

/// The name the program was run as, `program`, without its extension, or `job` when it has none.
fn program_name(program: Option<&OsString>) -> String {
    program
        .map(Path::new)
        .and_then(Path::file_stem)
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "job".to_string())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, OnceLock};

    use rstest::{fixture, rstest};
    use serde::{Deserialize, Serialize};
    use tracel_job::DefinitionsFile;

    use super::*;
    use crate::{BoxError, job_num};

    #[derive(Serialize, Deserialize)]
    #[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
    struct Epochs {
        epochs: u32,
    }

    #[fixture]
    fn jobs() -> Jobs {
        Jobs::new()
            .job("train", |_: Epochs| Ok::<_, BoxError>(()))
            .job("fail", |()| Err::<(), BoxError>("the job failed".into()))
    }

    /// What the command line `program <args>` asks of `jobs`.
    fn parse(jobs: &Jobs, args: &[&str]) -> Result<Request, RunError> {
        let mut command = command_line(jobs, "program");
        read_request(jobs, &mut command, ["program"].iter().chain(args))
    }

    fn arguments(args: &[&str]) -> impl Iterator<Item = OsString> {
        ["program"].iter().chain(args).map(OsString::from)
    }

    fn describing(path: &Path) -> Launch {
        Launch::new(Some(path.as_os_str().to_owned()), None)
    }

    fn run(jobs: Jobs, args: &[&str]) -> Result<(), RunError> {
        execute(&jobs, arguments(args), Launch::new(None, None))
    }

    #[test]
    fn an_empty_launch_variable_counts_as_unset() {
        let launch = Launch::new(Some(OsString::new()), Some(OsString::new()));

        assert_eq!((launch.describe, launch.num), (None, None));
    }

    #[rstest]
    #[case::no_job(&[])]
    #[case::a_job_that_fails(&["fail"])]
    #[case::an_unusable_command_line(&["--bogus"])]
    fn a_launcher_asking_for_the_definitions_gets_them_instead_of_a_run(
        jobs: Jobs,
        #[case] args: &[&str],
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.json");

        execute(&jobs, arguments(args), describing(&path)).unwrap();

        let file = DefinitionsFile::from_json(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(file.jobs, jobs.definitions().cloned().collect::<Vec<_>>());
    }

    #[rstest]
    fn definitions_that_cannot_be_written_fail_the_run(jobs: Jobs) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing").join("jobs.json");

        let failed = execute(&jobs, arguments(&["train"]), describing(&path)).unwrap_err();

        assert_eq!(failed.exit_code(), tracel_job::ExitCode::Failed);
    }

    #[rstest]
    #[case::numbered(Some("12"))]
    #[case::unnumbered(None)]
    fn the_job_is_numbered_as_its_launcher_numbers_it(#[case] num: Option<&str>) {
        let seen = Arc::new(OnceLock::new());
        let jobs = Jobs::new().job("train", {
            let seen = seen.clone();
            move |()| {
                let _ = seen.set(job_num());
                Ok::<_, BoxError>(())
            }
        });
        let launch = Launch::new(None, num.map(OsString::from));

        execute(&jobs, arguments(&["train"]), launch).unwrap();

        assert_eq!(seen.get(), Some(&num.map(str::to_string)));
    }

    #[rstest]
    fn the_named_job_runs_with_the_input_its_command_line_gives(jobs: Jobs) {
        let given = serde_json::json!({"epochs": 2});

        let request = parse(&jobs, &["train", &given.to_string()]);

        assert!(matches!(
            request,
            Ok(Request::Run { job, input }) if job == "train" && input == given
        ));
    }

    #[rstest]
    fn a_config_file_gives_the_named_job_its_input(jobs: Jobs) {
        let given = serde_json::json!({"epochs": 2});
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.json");
        std::fs::write(&config, given.to_string()).unwrap();

        let request = parse(&jobs, &["train", "-c", config.to_str().unwrap()]);

        assert!(matches!(
            request,
            Ok(Request::Run { job, input }) if job == "train" && input == given
        ));
    }

    #[rstest]
    fn the_default_job_runs_with_no_input_when_no_job_is_named(jobs: Jobs) {
        let request = parse(&jobs.default_job("train"), &[]);

        assert!(matches!(
            request,
            Ok(Request::Run { job, input }) if job == "train" && input.is_null()
        ));
    }

    #[rstest]
    fn an_unknown_job_is_rejected_with_the_registered_names(jobs: Jobs) {
        let rejected = parse(&jobs, &["infer"]);

        assert!(matches!(
            rejected,
            Err(RunError::UnknownJob { name, available })
                if name == "infer" && available == names(&jobs)
        ));
    }

    #[rstest]
    fn an_unknown_default_job_is_rejected_with_the_registered_names(jobs: Jobs) {
        let registered = names(&jobs);

        let rejected = run(jobs.default_job("infer"), &[]);

        assert!(matches!(
            rejected,
            Err(RunError::UnknownJob { name, available })
                if name == "infer" && available == registered
        ));
    }

    #[rstest]
    fn naming_no_job_without_a_default_is_rejected_with_the_registered_names(jobs: Jobs) {
        let rejected = parse(&jobs, &[]);

        assert!(matches!(
            rejected,
            Err(RunError::MissingJob { available }) if available == names(&jobs)
        ));
    }

    #[rstest]
    #[case::an_unknown_job(&["infer"], tracel_job::ExitCode::Rejected)]
    #[case::no_job(&[], tracel_job::ExitCode::Rejected)]
    #[case::an_unusable_command_line(&["train", "--bogus"], tracel_job::ExitCode::Rejected)]
    #[case::an_input_that_does_not_decode(
        &["train", r#"{"epochs": "ten"}"#],
        tracel_job::ExitCode::Rejected
    )]
    #[case::a_job_that_fails(&["fail"], tracel_job::ExitCode::Failed)]
    fn a_run_that_does_not_complete_exits_with_the_code_that_says_why(
        jobs: Jobs,
        #[case] args: &[&str],
        #[case] exit: tracel_job::ExitCode,
    ) {
        assert_eq!(run(jobs, args).unwrap_err().exit_code(), exit);
    }

    #[rstest]
    fn an_unreadable_config_file_is_rejected(jobs: Jobs) {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("missing.json");

        let rejected = run(jobs, &["train", "-c", config.to_str().unwrap()]).unwrap_err();

        assert_eq!(rejected.exit_code(), tracel_job::ExitCode::Rejected);
    }

    #[rstest]
    #[case::help(&["--help"])]
    #[case::version(&["--version"])]
    fn help_and_the_version_are_printed_instead_of_running_a_job(
        jobs: Jobs,
        #[case] args: &[&str],
    ) {
        let request = parse(&jobs.version("1.2.3"), args);

        assert!(matches!(request, Ok(Request::Print(_))));
    }

    #[rstest]
    fn the_completion_script_is_printed_for_the_shell_asked_for(jobs: Jobs) {
        let request = parse(&jobs, &["--completions", "bash"]);

        assert!(matches!(request, Ok(Request::Completions(Shell::Bash))));
    }

    #[rstest]
    fn without_a_version_there_is_no_version_flag(jobs: Jobs) {
        let request = parse(&jobs, &["--version"]);

        assert!(matches!(request, Err(RunError::Usage(_))));
    }

    #[cfg(feature = "schema")]
    #[test]
    fn a_job_with_an_input_schema_takes_its_fields_as_flags() {
        let jobs = Jobs::new().job(
            "train",
            crate::Job::new(|_: Epochs| Ok::<_, BoxError>(())).with_schema(),
        );
        let epochs = 2;

        let request = parse(&jobs, &["train", "--epochs", &epochs.to_string()]);

        assert!(matches!(
            request,
            Ok(Request::Run { input, .. }) if input == serde_json::json!({"epochs": epochs})
        ));
    }
}
