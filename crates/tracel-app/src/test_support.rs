use std::collections::HashMap;

use serde_json::Value;
use tracel_experiment::error::ExperimentError;
use tracel_experiment::{ExperimentProvider, ExperimentRun};

/// A provider for experiment jobs that are built or prepared, never run.
pub struct NeverRuns;

impl ExperimentProvider for NeverRuns {
    fn create_experiment(
        &self,
        name: String,
        _attributes: HashMap<String, Value>,
    ) -> Result<ExperimentRun, ExperimentError> {
        panic!("experiment '{name}' was not expected to run")
    }
}
