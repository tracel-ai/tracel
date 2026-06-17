use clap::{Parser, Subcommand};
use std::collections::HashMap;
use tracel_experiment::ExperimentJob;

use crate::{
    Mapper,
    error::CliError,
    job::{CliJob, DefaultJob, JobFunction, RegisteredJob},
};

#[derive(Parser)]
#[command(about = "Run a registered job")]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run an experiment job
    Experiment {
        /// Job name to run (uses default if omitted)
        job: Option<String>,
        /// Config string passed to the job's mapper
        config: Option<String>,
    },
    /// Run an inference job
    Inference {
        /// Job name to run (uses default if omitted)
        job: Option<String>,
        /// Config string passed to the job's mapper
        config: Option<String>,
    },
}

pub struct Cli {
    jobs: HashMap<String, RegisteredJob>,
    default: Option<DefaultJob>,
}

impl Cli {
    pub fn new() -> Self {
        Self {
            jobs: HashMap::new(),
            default: None,
        }
    }

    fn erase_job<J, I, O, F>(job: J, mapper: F) -> JobFunction
    where
        J: CliJob<I, O> + 'static,
        F: Mapper<I> + 'static,
        I: 'static,
        O: 'static,
    {
        Box::new(move |config_str: &str| {
            let input = mapper.map(config_str)?;
            job.execute(input).map(|_| ())
        })
    }

    pub fn register_exp<I, O, F>(mut self, job: ExperimentJob<I, O>, mapper: F) -> Self
    where
        F: Mapper<I> + 'static,
        I: 'static,
        O: 'static,
    {
        let name = job.name().to_string();
        let erased = Self::erase_job(job, mapper);
        self.jobs.insert(name, RegisteredJob::Experiment(erased));
        self
    }

    pub fn default<J, I, O>(mut self, job: J, config: I) -> Self
    where
        J: CliJob<I, O> + 'static,
        I: 'static,
        O: 'static,
    {
        self.default = Some(DefaultJob {
            runner: Box::new(move || job.execute(config).map(|_| ())),
        });
        self
    }

    pub fn run(self) -> Result<(), CliError> {
        let args = Args::parse();
        self.dispatch(args.command)
    }

    fn dispatch(self, command: Option<Command>) -> Result<(), CliError> {
        match command {
            Some(cmd) => {
                let (job_name, config_str) = match &cmd {
                    Command::Experiment { job, config }
                    | Command::Inference { job, config } => (
                        job.clone().unwrap_or_default(),
                        config.clone().unwrap_or_default(),
                    ),
                };

                let registered = self
                    .jobs
                    .get(&job_name)
                    .ok_or_else(|| CliError::UnknownJob {
                        name: job_name.clone(),
                        available: self.jobs.keys().cloned().collect(),
                    })?;

                let runner = match (registered, &cmd) {
                    (RegisteredJob::Experiment(f), Command::Experiment { .. }) => f,
                    (RegisteredJob::Inference(f), Command::Inference { .. }) => f,
                    _ => {
                        return Err(CliError::UnknownJob {
                            name: job_name,
                            available: self.jobs.keys().cloned().collect(),
                        });
                    }
                };

                runner(&config_str).map_err(CliError::JobError)
            }
            None => {
                let d = self.default.ok_or(CliError::MissingDefault)?;
                (d.runner)().map_err(CliError::JobError)
            }
        }
    }
}
