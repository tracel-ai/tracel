use tracel_experiment::ExperimentRun;
use tracel_experiment::error::{ExperimentError, ExperimentErrorKind};

use crate::backend::station::StationBackend;
use crate::experiment::ExperimentProvider;

impl ExperimentProvider for StationBackend {
    fn create_experiment(&self, name: String) -> Result<ExperimentRun, ExperimentError> {
        tracel_experiment::remote::create_station_experiment_run(self.client.clone(), name)
            .map_err(|e| ExperimentError {
                kind: ExperimentErrorKind::Internal,
                message: "Failed to start Station experiment run".to_string(),
                source: Some(e),
            })
    }
}
