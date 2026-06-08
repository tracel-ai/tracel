mod cloud;
mod local;
mod log_store;
mod session;
mod socket;
#[cfg(feature = "station")]
mod station;

// TODO: te3mporary re-export for the runtime crate, will be erased when we detach ourself completely from runtime
pub use cloud::create_cloud_experiment_run;

use std::error::Error;
use std::sync::Arc;

use tracel_experiment::ExperimentRun;
use tracel_experiment::ExperimentRunHandleExt;
use tracel_experiment::error::ExperimentError;
use tracel_experiment::error::ExperimentErrorKind;

pub trait ExperimentProvider: Send + Sync + 'static {
    fn create_experiment(&self, name: String) -> Result<ExperimentRun, ExperimentError>;
}

pub struct Experiment {
    provider: Arc<dyn ExperimentProvider>,
}

impl Experiment {
    pub(crate) fn new(provider: Arc<dyn ExperimentProvider>) -> Self {
        Self { provider }
    }

    pub fn create<T, F>(&self, name: &str, f: F) -> ExperimentJob<T>
    where
        F: Fn(&ExperimentRun, T) -> Result<(), Box<dyn Error>> + Send + Sync + 'static,
    {
        let provider = self.provider.clone();
        let name = name.to_string();

        let job_closure = move |input: T| -> Result<(), Box<dyn Error>> {
            validate_name(&name)?;

            let _ = tracel_experiment::integration::tracing::try_init_tracing_subscriber();

            let experiment = provider.create_experiment(name.clone())?;
            let handle = experiment.handle();
            let result = handle.in_scope(|| f(&experiment, input));

            match result {
                Ok(()) => {
                    experiment.finish()?;
                    Ok(())
                }
                Err(e) => {
                    let msg = e.to_string();
                    let _ = experiment.fail(msg);
                    Err(e)
                }
            }
        };

        ExperimentJob::new(job_closure)
    }
}

type JobFn<T> = dyn Fn(T) -> Result<(), Box<dyn std::error::Error>> + Send + Sync;

pub struct ExperimentJob<T> {
    job: Box<JobFn<T>>,
}

impl<T> ExperimentJob<T> {
    pub fn new<F>(f: F) -> Self
    where
        F: Fn(T) -> Result<(), Box<dyn std::error::Error>> + Send + Sync + 'static,
    {
        Self { job: Box::new(f) }
    }

    pub fn run(&self, input: T) -> Result<(), Box<dyn std::error::Error>> {
        (self.job)(input)
    }
}

fn validate_name(name: &str) -> Result<(), ExperimentError> {
    if name.is_empty() {
        return Err(ExperimentError::new(
            ExperimentErrorKind::Internal,
            "Experiment name must not be empty",
        ));
    }
    if name.len() > 128 {
        return Err(ExperimentError::new(
            ExperimentErrorKind::Internal,
            "Experiment name must not exceed 128 characters",
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(ExperimentError::new(
            ExperimentErrorKind::Internal,
            "Experiment name must contain only ASCII alphanumeric characters, hyphens, or underscores",
        ));
    }
    Ok(())
}
