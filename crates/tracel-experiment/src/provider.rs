use std::collections::HashMap;
use std::error::Error;
use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;

use crate::error::{ExperimentError, ExperimentErrorKind};
use crate::integration::tracing::try_init_tracing_subscriber;
use crate::{ExperimentRun, ExperimentRunHandleExt};

pub trait ExperimentProvider: Send + Sync + 'static {
    fn create_experiment(
        &self,
        name: String,
        attributes: HashMap<String, Value>,
    ) -> Result<ExperimentRun, ExperimentError>;
}

pub trait ExperimentFn<I, O>: Send + Sync {
    fn call(&self, run: &ExperimentRun, input: I) -> Result<O, Box<dyn Error + Send + Sync>>;
}

impl<I, O, F> ExperimentFn<I, O> for F
where
    F: Fn(&ExperimentRun, I) -> Result<O, Box<dyn Error + Send + Sync>> + Send + Sync,
{
    fn call(&self, run: &ExperimentRun, input: I) -> Result<O, Box<dyn Error + Send + Sync>> {
        (self)(run, input)
    }
}

pub struct ExperimentModule {
    provider: Arc<dyn ExperimentProvider>,
}

impl ExperimentModule {
    // On pourais rajouter des settings ici comme un builde d'experiment module
    pub fn new(provider: Arc<dyn ExperimentProvider>) -> Self {
        Self { provider }
    }

    pub fn create<I, O>(
        &self,
        name: &str,
        f: impl ExperimentFn<I, O> + 'static,
    ) -> Result<ExperimentJob<I, O>, ExperimentError> {
        ExperimentJob::new(self.provider.clone(), name.to_string(), f)
    }
}

pub struct ExperimentJob<I, O> {
    provider: Arc<dyn ExperimentProvider>,
    name: String,
    attributes: HashMap<String, Value>,
    f: Box<dyn ExperimentFn<I, O>>,
}

impl<I, O> ExperimentJob<I, O> {
    fn new<F>(
        provider: Arc<dyn ExperimentProvider>,
        name: String,
        f: F,
    ) -> Result<Self, ExperimentError>
    where
        F: ExperimentFn<I, O> + 'static,
    {
        validate_name(&name)?;

        Ok(Self {
            provider,
            name,
            attributes: HashMap::new(),
            f: Box::new(f),
        })
    }

    pub fn attribute(mut self, key: impl Into<String>, value: impl Serialize) -> Self {
        self.attributes.insert(
            key.into(),
            serde_json::to_value(value).expect("attribute value must be serializable"),
        );
        self
    }

    pub fn attributes(mut self, attrs: HashMap<String, Value>) -> Self {
        self.attributes.extend(attrs);
        self
    }

    pub fn run(&self, input: I) -> Result<O, Box<dyn std::error::Error + Send + Sync>> {
        let _ = try_init_tracing_subscriber();

        let experiment = self
            .provider
            .create_experiment(self.name.clone(), self.attributes.clone())?;
        experiment.cancel_token().cancel_on_ctrlc();
        let handle = experiment.handle();
        let result = handle.in_scope(|| self.f.call(&experiment, input));

        match result {
            Ok(output) => {
                experiment.finish()?;
                Ok(output)
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
