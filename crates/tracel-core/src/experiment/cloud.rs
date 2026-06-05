use crate::experiment::ExperimentProvider;
use tracel_experiment::ExperimentRun;
use tracel_experiment::error::{ExperimentError, ExperimentErrorKind};

use crate::backend::cloud::CloudBackend;

impl ExperimentProvider for CloudBackend {
    fn create_experiment(&self, name: String) -> Result<ExperimentRun, ExperimentError> {
        let digest = "46523358ec1646354ddab1cd8b93f2b920b44b24a26ea86c129d666d6bae2a5f".to_string();
        tracel_experiment::remote::create_cloud_experiment_run(
            self.client.clone(),
            &self.namespace,
            &self.project,
            digest,
            name,
        )
        .map_err(|e| ExperimentError {
            kind: ExperimentErrorKind::Internal,
            message: "Failed to start Cloud experiment run".to_string(),
            source: Some(Box::new(e)),
        })
    }
}
