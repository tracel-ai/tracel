use std::{any::Any, error::Error};
use tracel_experiment::ExperimentJob;

pub type ValidateFn =
    Box<dyn Fn(&str) -> Result<Box<dyn Any + Send>, Box<dyn Error + Send + Sync>> + Send + Sync>;
pub type RunFn =
    Box<dyn Fn(Box<dyn Any + Send>) -> Result<(), Box<dyn Error + Send + Sync>> + Send + Sync>;

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
