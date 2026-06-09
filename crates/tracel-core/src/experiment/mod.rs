mod local;
mod remote;

// TODO: te3mporary re-export for the runtime crate, will be erased when we detach ourself completely from runtime
pub use remote::cloud::create_cloud_experiment_run;
use serde::Serialize;

use std::collections::HashMap;
use std::error::Error;
use std::sync::Arc;

use serde_json::Value;
use tracel_experiment::ExperimentRun;
use tracel_experiment::ExperimentRunHandleExt;
use tracel_experiment::error::ExperimentError;
use tracel_experiment::error::ExperimentErrorKind;

pub trait ExperimentProvider: Send + Sync + 'static {
    fn create_experiment(
        &self,
        name: String,
        attributes: HashMap<String, Value>,
    ) -> Result<ExperimentRun, ExperimentError>;
}

pub struct Experiment {
    provider: Arc<dyn ExperimentProvider>,
}

impl Experiment {
    pub(crate) fn new(provider: Arc<dyn ExperimentProvider>) -> Self {
        Self { provider }
    }

    pub fn create_job<T, F>(&self, name: &str, f: F) -> ExperimentJob<T>
    where
        F: Fn(&ExperimentRun, T) -> Result<(), Box<dyn Error>> + Send + Sync + 'static,
    {
        ExperimentJob::new(self.provider.clone(), name.to_string(), f)
    }
}

type UserFn<T> = dyn Fn(&ExperimentRun, T) -> Result<(), Box<dyn std::error::Error>> + Send + Sync;

pub struct ExperimentJob<T> {
    provider: Arc<dyn ExperimentProvider>,
    name: String,
    attributes: HashMap<String, Value>,
    f: Box<UserFn<T>>,
}

impl<T> ExperimentJob<T> {
    fn new<F>(provider: Arc<dyn ExperimentProvider>, name: String, f: F) -> Self
    where
        F: Fn(&ExperimentRun, T) -> Result<(), Box<dyn std::error::Error>> + Send + Sync + 'static,
    {
        Self {
            provider,
            name,
            attributes: HashMap::new(),
            f: Box::new(f),
        }
    }

    pub fn attribute(mut self, key: impl Into<String>, value: impl Serialize) -> Self {
        self.attributes.insert(
            key.into(),
            serde_json::to_value(value).expect("attribute value must be serializable"),
        );
        self
    }

    pub fn run(&self, input: T) -> Result<(), Box<dyn std::error::Error>> {
        validate_name(&self.name)?;

        let _ = tracel_experiment::integration::tracing::try_init_tracing_subscriber();

        let experiment = self
            .provider
            .create_experiment(self.name.clone(), self.attributes.clone())?;
        let handle = experiment.handle();
        let result = handle.in_scope(|| (self.f)(&experiment, input));

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
