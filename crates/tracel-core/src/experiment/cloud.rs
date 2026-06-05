use burn_central_client::websocket::WebSocketError;
use burn_central_client::{Client, ClientError};

use tracel_experiment::error::{ExperimentError, ExperimentErrorKind};
use tracel_experiment::remote::base::RemoteExperimentSession;
use tracel_experiment::remote::cloud::{
    ConsoleArtifactReader, ConsoleArtifactUploader, ConsoleLogUploader, ExperimentPath,
};
use tracel_experiment::{CancelToken, ExperimentId, ExperimentRun};

use crate::backend::cloud::CloudBackend;
use crate::experiment::ExperimentProvider;

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub(crate) enum ConsoleError {
    Http(#[from] ClientError),
    WebSocket(#[from] WebSocketError),
}

impl ExperimentProvider for CloudBackend {
    fn create_experiment(&self, name: String) -> Result<ExperimentRun, ExperimentError> {
        let digest = "46523358ec1646354ddab1cd8b93f2b920b44b24a26ea86c129d666d6bae2a5f".to_string();
        create_run(
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

pub fn create_cloud_experiment_run(
    client: Client,
    namespace: &str,
    project_name: &str,
    digest: String,
    routine: String,
) -> Result<ExperimentRun, Box<dyn std::error::Error + Send + Sync>> {
    Ok(create_run(
        client,
        namespace,
        project_name,
        digest,
        routine,
    )?)
}

fn create_run(
    client: Client,
    namespace: &str,
    project_name: &str,
    digest: String,
    routine: String,
) -> Result<ExperimentRun, ConsoleError> {
    let experiment = client.create_experiment(namespace, project_name, None, digest, routine)?;

    let experiment_num = experiment.experiment_num;
    let path = ExperimentPath::new(namespace, project_name, experiment_num);
    let cancel_token = CancelToken::new();

    let log_uploader = ConsoleLogUploader::new(client.clone(), path.clone());
    let artifact_uploader = ConsoleArtifactUploader::new(client.clone(), path.clone());

    let ws = client.create_experiment_run_websocket(namespace, project_name, experiment_num)?;

    let session = RemoteExperimentSession::new(
        Box::new(log_uploader),
        Box::new(artifact_uploader),
        ws,
        cancel_token.clone(),
    );

    let reader = ConsoleArtifactReader::new(client, path);
    let id = ExperimentId::from(format!("{}", experiment_num));

    Ok(ExperimentRun::new(id, session, reader, cancel_token))
}
