use burn_central_client::station::experiment::CreateExperimentRequest;
use burn_central_client::StationClient;

use tracel_experiment::remote::base::RemoteExperimentSession;
use tracel_experiment::remote::station::{
    ExperimentPath, StationArtifactReader, StationArtifactUploader, StationLogUploader,
};
use tracel_experiment::error::{ExperimentError, ExperimentErrorKind};
use tracel_experiment::{CancelToken, ExperimentId, ExperimentRun};

use crate::backend::station::{StationBackend, StationError};
use crate::experiment::ExperimentProvider;

impl ExperimentProvider for StationBackend {
    fn create_experiment(&self, name: String) -> Result<ExperimentRun, ExperimentError> {
        create_run(self.client.clone(), name).map_err(|e| ExperimentError {
            kind: ExperimentErrorKind::Internal,
            message: "Failed to start Station experiment run".to_string(),
            source: Some(Box::new(e)),
        })
    }
}

fn create_run(client: StationClient, routine: String) -> Result<ExperimentRun, StationError> {
    let experiments_client = client.experiments();
    let experiment = experiments_client.create(CreateExperimentRequest {
        description: None,
        routine_run: routine,
    })?;

    let experiment_num = experiment.experiment_num;
    let path = ExperimentPath::new(experiment_num);
    let cancel_token = CancelToken::new();

    let log_uploader = StationLogUploader::new(client.clone(), path.clone());
    let artifact_uploader = StationArtifactUploader::new(client.clone(), path);

    let ws = experiments_client.create_run_websocket(experiment_num)?;

    let session = RemoteExperimentSession::new(
        Box::new(log_uploader),
        Box::new(artifact_uploader),
        ws,
        cancel_token.clone(),
    );

    let reader = StationArtifactReader::new(client);
    let id = ExperimentId::from(format!("{}", experiment_num));

    Ok(ExperimentRun::new(id, session, reader, cancel_token))
}
