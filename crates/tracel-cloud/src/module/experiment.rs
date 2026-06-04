use std::error::Error;
use std::sync::Arc;

use tracel_experiment::ExperimentJob;
use tracel_experiment::ExperimentRun;
use tracel_experiment::ExperimentRunHandleExt;
use tracel_experiment::error::ExperimentError;

pub trait ExperimentProvider: Send + Sync + 'static {
    fn setup_experiment(&self, routine: String) -> Result<ExperimentRun, ExperimentError>;
}

pub struct Experiment {
    provider: Arc<dyn ExperimentProvider>,
}

impl Experiment {
    pub(crate) fn new(provider: Arc<dyn ExperimentProvider>) -> Self {
        Self { provider }
    }

    pub fn create<T, F>(&self, f: F) -> ExperimentJob<T>
    where
        F: Fn(&ExperimentRun, T) -> Result<(), Box<dyn Error>> + Send + Sync + 'static,
    {
        let provider = self.provider.clone();
        let job_closure = move |input: T| {
            let _ = tracel_experiment::integration::tracing::try_init_tracing_subscriber();

            let experiment = provider.setup_experiment(std::any::type_name::<F>().to_string())?;
            let handle = experiment.handle();
            let result = handle.in_scope(|| f(&experiment, input));

            match result {
                Ok(()) => experiment
                    .finish()
                    .map_err(|e| format!("Failed to finish experiment: {e}").into()),
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
