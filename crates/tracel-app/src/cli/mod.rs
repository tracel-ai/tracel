mod error;

pub use error::CliError;

use crate::{job::Job, job_register::JobRegister, mapper::Mapper};
use clap::Parser;
use std::error::Error;

#[derive(Parser)]
#[command(about = "Run a registered job")]
struct Args {
    job: Option<String>,
    config: Option<String>,
}

struct DefaultJob {
    runner: Box<dyn FnOnce() -> Result<(), Box<dyn Error + Send + Sync>>>,
}

#[derive(Default)]
pub struct Cli {
    register: JobRegister,
    default: Option<DefaultJob>,
}

impl Cli {
    pub fn new() -> Self {
        Self {
            register: JobRegister::new(),
            default: None,
        }
    }

    pub fn register<J, I, O, F>(mut self, job: J, mapper: F) -> Self
    where
        J: Job<I, O> + 'static,
        F: Mapper<I> + 'static,
        I: 'static,
        O: 'static,
    {
        self.register = self.register.register(job, mapper);
        self
    }

    pub fn default_job<J, I, O>(mut self, job: J, config: I) -> Self
    where
        J: Job<I, O> + 'static,
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
                let config_str = config.unwrap_or_default();
                self.register
                    .dispatch(&job_name, &config_str)
                    .map_err(CliError::JobError)
            }
            None => {
                let d = self.default.ok_or(CliError::MissingDefault)?;
                (d.runner)().map_err(CliError::JobError)
            }
        }
    }
}
