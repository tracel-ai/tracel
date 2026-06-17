use std::error::Error;
use tracel_experiment::ExperimentJob;

pub type JobFunction = Box<dyn Fn(&str) -> Result<(), Box<dyn Error + Send + Sync>>>;

pub struct DefaultJob {
    pub runner: Box<dyn FnOnce() -> Result<(), Box<dyn Error + Send + Sync>>>,
}

pub trait CliJob<I, O> {
    fn name(&self) -> &str;
    fn execute(&self, input: I) -> Result<O, Box<dyn Error + Send + Sync>>;
}

impl<I, O> CliJob<I, O> for ExperimentJob<I, O> {
    fn name(&self) -> &str {
        self.name()
    }

    fn execute(&self, input: I) -> Result<O, Box<dyn Error + Send + Sync>> {
        self.run(input)
    }
}
