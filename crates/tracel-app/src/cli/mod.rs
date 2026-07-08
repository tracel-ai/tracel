mod command;
mod error;
/// Config mappers that turn a CLI string argument into a typed input (CLI-only).
pub mod mapper;

pub use command::{CliCommand, IntoCliCommand};
pub use error::CliError;

use clap::Parser;
use std::collections::HashMap;
use std::error::Error;
use tracel_experiment::ExperimentJob;

#[derive(Parser)]
#[command(about = "Run a registered command")]
struct Args {
    command: Option<String>,
    config: Option<String>,
}

struct DefaultCommand {
    runner: Box<dyn FnOnce() -> Result<(), Box<dyn Error + Send + Sync>>>,
}

#[derive(Default)]
pub struct Cli {
    commands: HashMap<String, Box<dyn CliCommand>>,
    default: Option<DefaultCommand>,
}

impl Cli {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a bespoke [`CliCommand`]. [`register`](Self::register) builds on this for capability
    /// jobs; use this directly only for a custom command.
    pub fn command<C>(self, command: C) -> Self
    where
        C: CliCommand + 'static,
    {
        self.command_boxed(Box::new(command))
    }

    fn command_boxed(mut self, command: Box<dyn CliCommand>) -> Self {
        let name = command.name().to_string();
        if self.commands.contains_key(&name) {
            panic!("command '{name}' is already registered");
        }
        self.commands.insert(name, command);
        self
    }

    /// Register a capability job (experiment, inference, ...), decoding its input with `mapper`.
    ///
    /// The same call works for any job type that implements [`IntoCliCommand`].
    pub fn register<T, M>(self, job: T, mapper: M) -> Self
    where
        T: IntoCliCommand<M>,
    {
        self.command_boxed(job.into_cli_command(mapper))
    }

    /// Set the experiment job to run when no command name is given, with a preset config.
    pub fn default_job<I, O>(mut self, job: ExperimentJob<I, O>, config: I) -> Self
    where
        I: Send + 'static,
        O: 'static,
    {
        self.default = Some(DefaultCommand {
            runner: Box::new(move || job.run(config).map(|_| ())),
        });
        self
    }

    pub fn run(self) -> Result<(), CliError> {
        let args = Args::parse();
        self.dispatch(args.command, args.config)
    }

    fn dispatch(self, command: Option<String>, config: Option<String>) -> Result<(), CliError> {
        match command {
            Some(name) => {
                let config_str = config.unwrap_or_default();
                let command =
                    self.commands
                        .get(&name)
                        .ok_or_else(|| CliError::UnknownCommand {
                            name: name.clone(),
                            available: self.commands.keys().cloned().collect(),
                        })?;
                command.run(&config_str)
            }
            None => {
                let d = self.default.ok_or(CliError::MissingDefault)?;
                (d.runner)().map_err(CliError::ExecutionFailed)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    struct FakeCommand {
        name: &'static str,
        fail: bool,
        ran: Arc<AtomicBool>,
    }

    impl FakeCommand {
        fn new(name: &'static str) -> Self {
            Self {
                name,
                fail: false,
                ran: Arc::new(AtomicBool::new(false)),
            }
        }

        fn failing(name: &'static str) -> Self {
            Self {
                name,
                fail: true,
                ran: Arc::new(AtomicBool::new(false)),
            }
        }
    }

    impl CliCommand for FakeCommand {
        fn name(&self) -> &str {
            self.name
        }

        fn run(&self, _config: &str) -> Result<(), CliError> {
            self.ran.store(true, Ordering::SeqCst);
            if self.fail {
                Err(CliError::ExecutionFailed("boom".into()))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn given_registered_command_when_dispatching_by_name_then_runs_it() {
        let cli = Cli::new().command(FakeCommand::new("train"));
        assert!(cli.dispatch(Some("train".into()), Some("{}".into())).is_ok());
    }

    #[test]
    fn given_unknown_command_when_dispatching_then_returns_unknown_command_error() {
        let cli = Cli::new().command(FakeCommand::new("train"));
        let result = cli.dispatch(Some("infer".into()), None);
        assert!(matches!(result, Err(CliError::UnknownCommand { .. })));
    }

    #[test]
    fn given_failing_command_when_dispatching_then_returns_execution_failed() {
        let cli = Cli::new().command(FakeCommand::failing("train"));
        let result = cli.dispatch(Some("train".into()), None);
        assert!(matches!(result, Err(CliError::ExecutionFailed(_))));
    }

    #[test]
    fn given_no_command_and_no_default_when_dispatching_then_returns_missing_default() {
        let cli = Cli::new();
        assert!(matches!(cli.dispatch(None, None), Err(CliError::MissingDefault)));
    }

    #[test]
    #[should_panic(expected = "already registered")]
    fn given_duplicate_command_name_when_registering_then_panics() {
        Cli::new()
            .command(FakeCommand::new("train"))
            .command(FakeCommand::new("train"));
    }
}
