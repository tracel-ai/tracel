use clap::Parser;
use std::{collections::HashMap, error::Error};
use tracel_experiment::ExperimentJob;

use crate::{error::CliError, job::CliJob};

type JobFunction = Box<dyn Fn(&str) -> Result<(), Box<dyn Error + Send + Sync>>>;

#[derive(Parser)]
#[command(about = "Run a registered experiment job")]
struct Args {
    /// Job name to run (uses default if omitted)
    job: Option<String>,
    /// Config string passed to the job's mapper
    config: Option<String>,
}

pub struct Cli {
    jobs: HashMap<String, JobFunction>,
    default: Option<String>,
}

impl Cli {
    pub fn new() -> Self {
        Self {
            jobs: HashMap::new(),
            default: None,
        }
    }

    fn register<J, I, O, F>(mut self, name: &str, job: J, mapper: F) -> Self
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
        self.jobs.insert(name.to_string(), erased);
        self
    }

    /// Convenience wrapper for ExperimentJob. Calls register() internally.
    pub fn register_exp<I, O, F>(self, name: &str, job: ExperimentJob<I, O>, mapper: F) -> Self
    where
        F: Fn(&str) -> Result<I, Box<dyn Error + Send + Sync>> + 'static,
        I: 'static,
        O: 'static,
    {
        self.register(name, job, mapper)
    }

    /// Set the job that runs when no job name is given on the CLI.
    pub fn default(mut self, name: &str) -> Self {
        self.default = Some(name.to_string());
        self
    }

    /// Parse CLI args and dispatch to the matching job.
    pub fn run(self) -> Result<(), CliError> {
        let args = Args::parse();
        self.dispatch(args.job.as_deref(), args.config.as_deref())
    }

    fn dispatch(self, job: Option<&str>, config: Option<&str>) -> Result<(), CliError> {
        let job_name = match job {
            Some(j) => j.to_string(),
            None => self.default.as_deref().ok_or(CliError::MissingDefault)?.to_string(),
        };
        let config_str = config.unwrap_or("");

        let runner = self
            .jobs
            .get(&job_name)
            .ok_or_else(|| CliError::UnknownJob {
                name: job_name.clone(),
                available: self.jobs.keys().cloned().collect(),
            })?;

        runner(config_str).map_err(CliError::JobError)
    }
}
