use crate::{
    Mapper,
    error::CliError,
    job::{CliJob, DefaultJob, JobFunction},
};
use clap::Parser;
use std::collections::HashMap;

#[derive(Parser)]
#[command(about = "Run a registered job")]
struct Args {
    job: Option<String>,
    config: Option<String>,
}

#[derive(Default)]
pub struct Cli {
    jobs: HashMap<String, JobFunction>,
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
            let input = mapper.map(config_str).map_err(CliError::ConfigError)?;
            job.execute(input).map(|_| ()).map_err(CliError::JobError)
        })
    }

    pub fn register<J, I, O, F>(mut self, job: J, mapper: F) -> Self
    where
        J: CliJob<I, O> + 'static,
        F: Mapper<I> + 'static,
        I: 'static,
        O: 'static,
    {
        let name = job.name().to_string();
        let erased = Self::erase_job(job, mapper);
        self.jobs.insert(name, erased);
        self
    }

    pub fn default_job<J, I, O>(mut self, job: J, config: I) -> Self
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
        self.dispatch(args.job, args.config)
    }

    fn dispatch(self, job: Option<String>, config: Option<String>) -> Result<(), CliError> {
        match job {
            Some(job_name) => {
                let runner = self
                    .jobs
                    .get(&job_name)
                    .ok_or_else(|| CliError::UnknownJob {
                        name: job_name.clone(),
                        available: self.jobs.keys().cloned().collect(),
                    })?;

                let config_str = config.unwrap_or_default();
                runner(&config_str)
            }
            None => {
                let d = self.default.ok_or(CliError::MissingDefault)?;
                (d.runner)().map_err(CliError::JobError)
            }
        }
    }
}
