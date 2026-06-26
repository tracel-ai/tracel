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
    // TODO: Add settings here (e.g., an ExperimentModule builder).
    pub fn new(provider: Arc<dyn ExperimentProvider>) -> Self {
        Self { provider }
    }

    pub fn create<I, O>(
        &self,
        name: &str,
        f: impl ExperimentFn<I, O> + 'static,
    ) -> ExperimentJob<I, O> {
        ExperimentJob::new(self.provider.clone(), name.to_string(), f)
    }
}

#[derive(Clone)]
pub struct ExperimentJob<I, O> {
    provider: Arc<dyn ExperimentProvider>,
    name: String,
    attributes: HashMap<String, Value>,
    f: Arc<dyn ExperimentFn<I, O>>,
}

impl<I, O> ExperimentJob<I, O> {
    fn new<F>(provider: Arc<dyn ExperimentProvider>, name: String, f: F) -> Self
    where
        F: ExperimentFn<I, O> + 'static,
    {
        Self {
            provider,
            name,
            attributes: HashMap::new(),
            f: Arc::new(f),
        }
    }

    #[doc(hidden)]
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn attribute(
        mut self,
        key: impl Into<String>,
        value: impl Serialize,
    ) -> Result<Self, ExperimentError> {
        let value = serde_json::to_value(value).map_err(|e| {
            ExperimentError::with_source(
                ExperimentErrorKind::Internal,
                "Failed to serialize experiment attribute",
                e,
            )
        })?;

        self.attributes.insert(key.into(), value);
        Ok(self)
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
