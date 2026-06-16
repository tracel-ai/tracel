use clap::{Parser, Subcommand};
use std::{collections::HashMap, error::Error};
use tracel_experiment::ExperimentJob;

use crate::{error::CliError, job::CliJob};

type JobFunction = Box<dyn Fn(&str) -> Result<(), Box<dyn Error + Send + Sync>>>;

#[derive(Parser)]
#[command(about = "Run a registered job")]
struct Args {
    #[command(subcommand)]
    command: Command,
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
    experiment_jobs: HashMap<String, JobFunction>,
    inference_jobs: HashMap<String, JobFunction>,
    default: Option<String>,
}

impl Cli {
    pub fn new() -> Self {
        Self {
            experiment_jobs: HashMap::new(),
            inference_jobs: HashMap::new(),
            default: None,
        }
    }

    fn insert_job<J, I, O, F>(jobs: &mut HashMap<String, JobFunction>, name: &str, job: J, mapper: F)
    where
        J: CliJob<I, O> + 'static,
        F: Fn(&str) -> Result<I, Box<dyn Error + Send + Sync>> + 'static,
        I: 'static,
        O: 'static,
    {
        let erased = Box::new(move |config_str: &str| {
            let input = mapper(config_str)?;
            job.execute(input).map(|_| ())
        });
        jobs.insert(name.to_string(), erased);
    }

    pub fn register_exp<I, O, F>(mut self, job: ExperimentJob<I, O>, mapper: F) -> Self
    where
        F: Fn(&str) -> Result<I, Box<dyn Error + Send + Sync>> + 'static,
        I: 'static,
        O: 'static,
    {
        let name = job.name().to_string();
        Self::insert_job(&mut self.experiment_jobs, &name, job, mapper);
        self
    }

    /// Set the job that runs when no job name is given on the CLI.
    pub fn default(mut self, name: &str) -> Self {
        self.default = Some(name.to_string());
        self
    }

    /// Parse CLI args and dispatch to the matching job.
    pub fn run(self) -> Result<(), CliError> {
        let args = Args::parse();
        self.dispatch(args.command)
    }

    fn dispatch(self, command: Command) -> Result<(), CliError> {
        let (jobs, job, config) = match command {
            Command::Experiment { job, config } => (&self.experiment_jobs, job, config),
            Command::Inference { job, config } => (&self.inference_jobs, job, config),
        };

        let job_name = match job {
            Some(j) => j,
            None => self
                .default
                .as_deref()
                .ok_or(CliError::MissingDefault)?
                .to_string(),
        };
        let config_str = config.as_deref().unwrap_or("");

        let runner = jobs
            .get(&job_name)
            .ok_or_else(|| CliError::UnknownJob {
                name: job_name.clone(),
                available: jobs.keys().cloned().collect(),
            })?;

        runner(config_str).map_err(CliError::JobError)
    }
}
