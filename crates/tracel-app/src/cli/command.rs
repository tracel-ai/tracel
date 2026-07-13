use serde::Serialize;

use tracel_experiment::ExperimentJob;
use tracel_inference::InferenceJob;

use crate::cli::error::CliError;
use crate::cli::mapper::Mapper;

/// A capability that the CLI can run from a string config.
///
/// This is the CLI's local trait. You rarely implement it directly — [`Cli::register`] accepts any
/// capability job (experiment, inference, ...) via [`IntoCliCommand`]. Implement it only for a
/// bespoke command that neither capability covers.
///
/// [`Cli::register`]: crate::cli::Cli::register
pub trait CliCommand: Send + Sync {
    /// The name used to select this command.
    fn name(&self) -> &str;
    /// Run the command with the given raw config string.
    fn run(&self, config: &str) -> Result<(), CliError>;
}

/// Turns a capability job plus a config mapper into a [`CliCommand`].
///
/// Implemented for `ExperimentJob` and `InferenceJob`, so `Cli::register(job, mapper)` works
/// uniformly for either. A new capability becomes CLI-registrable by implementing this for its job.
pub trait IntoCliCommand<M> {
    fn into_cli_command(self, mapper: M) -> Box<dyn CliCommand>;
}

/// Runs an [`ExperimentJob`] from the CLI: parse the config, run the job to completion.
struct ExperimentCliCommand<I, O, M> {
    job: ExperimentJob<I, O>,
    mapper: M,
}

impl<I, O, M> CliCommand for ExperimentCliCommand<I, O, M>
where
    I: Send + 'static,
    O: 'static,
    M: Mapper<I> + Send + Sync,
{
    fn name(&self) -> &str {
        self.job.name()
    }

    fn run(&self, config: &str) -> Result<(), CliError> {
        let input = self
            .mapper
            .map(config)
            .map_err(CliError::ValidationFailed)?;
        self.job
            .run(input)
            .map(|_| ())
            .map_err(CliError::ExecutionFailed)
    }
}

impl<I, O, M> IntoCliCommand<M> for ExperimentJob<I, O>
where
    I: Send + 'static,
    O: 'static,
    M: Mapper<I> + Send + Sync + 'static,
{
    fn into_cli_command(self, mapper: M) -> Box<dyn CliCommand> {
        Box::new(ExperimentCliCommand { job: self, mapper })
    }
}

/// Runs an [`InferenceJob`] from the CLI: parse the config, run once, print each output as an
/// NDJSON line to stdout.
struct InferenceCliCommand<I, O, M> {
    job: InferenceJob<I, O>,
    mapper: M,
}

impl<I, O, M> CliCommand for InferenceCliCommand<I, O, M>
where
    I: Send + 'static,
    O: Serialize + Send + Sync + 'static,
    M: Mapper<I> + Send + Sync,
{
    fn name(&self) -> &str {
        self.job.name()
    }

    fn run(&self, config: &str) -> Result<(), CliError> {
        let input = self
            .mapper
            .map(config)
            .map_err(CliError::ValidationFailed)?;
        let stream = self
            .job
            .stream_once(input)
            .map_err(|e| CliError::ExecutionFailed(Box::new(e)))?;
        for item in stream {
            let output = item.map_err(CliError::ExecutionFailed)?;
            let line = serde_json::to_string(&output)
                .map_err(|e| CliError::ExecutionFailed(Box::new(e)))?;
            println!("{line}");
        }
        Ok(())
    }
}

impl<I, O, M> IntoCliCommand<M> for InferenceJob<I, O>
where
    I: Send + 'static,
    O: Serialize + Send + Sync + 'static,
    M: Mapper<I> + Send + Sync + 'static,
{
    fn into_cli_command(self, mapper: M) -> Box<dyn CliCommand> {
        Box::new(InferenceCliCommand { job: self, mapper })
    }
}
