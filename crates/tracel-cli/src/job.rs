use std::error::Error;
use tracel_experiment::ExperimentJob;

pub trait CliJob<I, O> {
    fn execute(&self, input: I) -> Result<O, Box<dyn Error + Send + Sync>>;
}

impl<I, O> CliJob<I, O> for ExperimentJob<I, O> {
    fn execute(&self, input: I) -> Result<O, Box<dyn Error + Send + Sync>> {
        self.run(input)
    }
}
