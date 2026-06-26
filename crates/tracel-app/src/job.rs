use std::error::Error;
use tracel_experiment::ExperimentJob;

pub type JobFunction = Box<dyn Fn(&str) -> Result<(), Box<dyn Error + Send + Sync>>>;

pub trait Job<I, O> {
    fn name(&self) -> &str;
    fn execute(&self, input: I) -> Result<O, Box<dyn Error + Send + Sync>>;
}

impl<I, O> Job<I, O> for ExperimentJob<I, O> {
    fn name(&self) -> &str {
        self.name()
    }

    fn execute(&self, input: I) -> Result<O, Box<dyn Error + Send + Sync>> {
        self.run(input)
    }
}
