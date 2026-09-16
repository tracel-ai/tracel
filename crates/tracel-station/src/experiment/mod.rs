mod artifacts;

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use serde_json::Value;
use tracel_artifact::bundle::FsBundle;
use tracel_artifact::download::{ArtifactDownloadFile, DownloadError, download_into};
use tracel_artifact::upload::{
    MultipartUploadFile, MultipartUploadPart, UploadError, upload_multipart,
};
use tracel_client::ClientError;
use tracel_client::station::experiment::{
    ArtifactFileSpecRequest, ArtifactResponse, CompleteUploadRequest, CreateArtifactRequest,
    CreateExperimentRequest, ListArtifactsQuery,
};
use tracel_client::websocket::WebSocketError;
use tracel_experiment::error::{ExperimentError, ExperimentErrorKind};
use tracel_experiment::{
    ArtifactKind, CancelToken, ExperimentId, ExperimentProvider, ExperimentRun,
    ExperimentRunControl,
};
use tracel_experiment_remote::{RemoteExperimentSession, SocketHandle};
use tracel_task::Job;

use self::artifacts::{StationArtifactReader, StationArtifactUploader};
use crate::station::StationInner;

#[derive(Debug, thiserror::Error)]
enum RunError {
    #[error("Failed to create experiment on Station: check your Station URL and connectivity")]
    ExperimentCreation(#[from] ClientError),
    #[error("Failed to establish WebSocket connection to Station")]
    WebSocket(#[from] WebSocketError),
}

#[derive(Debug, Clone)]
pub struct ExperimentPath {
    experiment_num: i32,
}

impl ExperimentPath {
    pub fn new(experiment_num: i32) -> Self {
        Self { experiment_num }
    }

    pub fn experiment_num(&self) -> i32 {
        self.experiment_num
    }
}

/// A scope for artifact operations within a specific experiment.
#[derive(Clone)]
pub struct ExperimentArtifactClient {
    station: Arc<StationInner>,
    exp_path: ExperimentPath,
}

impl ExperimentArtifactClient {
    pub fn new(station: Arc<StationInner>, exp_path: ExperimentPath) -> Self {
        Self { station, exp_path }
    }

    pub async fn upload(
        &self,
        name: String,
        kind: ArtifactKind,
        bundle: FsBundle,
    ) -> Result<String, ArtifactError> {
        let client = self.station.client.experiments();

        let mut specs = Vec::with_capacity(bundle.files().len());
        for file in bundle.files() {
            let size_bytes = file.size_bytes.ok_or_else(|| {
                ArtifactError::Internal(format!("Missing file size for {}", file.rel_path))
            })?;
            let checksum = file.checksum.clone().ok_or_else(|| {
                ArtifactError::Internal(format!("Missing checksum for {}", file.rel_path))
            })?;
            specs.push(ArtifactFileSpecRequest {
                rel_path: file.rel_path.clone(),
                size_bytes,
                checksum,
            });
        }

        let created = client
            .create_artifact(
                self.exp_path.experiment_num(),
                CreateArtifactRequest {
                    name,
                    kind: artifact_kind_name(kind).to_string(),
                    files: specs,
                },
            )
            .await?;

        let mut multipart_map = BTreeMap::new();
        for file in &created.files {
            multipart_map.insert(file.rel_path.clone(), &file.urls);
        }

        let mut uploads = Vec::with_capacity(bundle.files().len());

        for file in bundle.files() {
            let multipart_info = multipart_map.get(&file.rel_path).ok_or_else(|| {
                ArtifactError::Internal(format!(
                    "Missing multipart upload info for file {}",
                    file.rel_path
                ))
            })?;

            let parts = multipart_info
                .parts
                .iter()
                .map(|part| MultipartUploadPart {
                    part: part.part,
                    url: part.url.clone(),
                    size_bytes: part.size_bytes,
                })
                .collect::<Vec<_>>();

            uploads.push(MultipartUploadFile {
                rel_path: file.rel_path.clone(),
                parts,
            });
        }
        upload_multipart(&self.station.transfer, &bundle, &uploads, &mut ()).await?;

        client
            .complete_artifact_upload(
                self.exp_path.experiment_num(),
                &created.id,
                CompleteUploadRequest { file_names: None },
            )
            .await?;

        Ok(created.id)
    }

    /// Download an artifact as a filesystem-backed bundle, with its listing.
    pub async fn download(
        &self,
        name: &str,
    ) -> Result<(ArtifactResponse, FsBundle), ArtifactError> {
        let artifact = self.fetch(name).await?;
        let client = self.station.client.experiments();
        let presigned = client
            .presign_artifact_download(self.exp_path.experiment_num(), artifact.id.to_string())
            .await?;

        let mut files = Vec::with_capacity(presigned.files.len());
        for file in presigned.files {
            files.push(ArtifactDownloadFile {
                rel_path: file.rel_path,
                url: file.url,
                size_bytes: None,
                checksum: None,
            });
        }

        let mut bundle = FsBundle::temp().map_err(|error| {
            ArtifactError::Internal(format!("Failed to create temp bundle: {error}"))
        })?;
        download_into(&self.station.transfer, &mut bundle, &files, &mut ()).await?;

        Ok((artifact, bundle))
    }

    /// Fetch information about an artifact by name.
    pub async fn fetch(&self, name: &str) -> Result<ArtifactResponse, ArtifactError> {
        self.station
            .client
            .experiments()
            .list_artifacts(
                self.exp_path.experiment_num(),
                ListArtifactsQuery {
                    name: Some(name.to_string()),
                },
            )
            .await?
            .items
            .into_iter()
            .next()
            .ok_or_else(|| ArtifactError::NotFound(name.to_owned()))
    }
}

fn artifact_kind_name(kind: ArtifactKind) -> &'static str {
    match kind {
        ArtifactKind::Model => "model",
        ArtifactKind::Log => "log",
        ArtifactKind::Other => "other",
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ArtifactError {
    #[error("Artifact not found: {0}")]
    NotFound(String),
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error(transparent)]
    Download(#[from] DownloadError),
    #[error(transparent)]
    Upload(#[from] UploadError),
    #[error("Internal error: {0}")]
    Internal(String),
}

pub struct StationExperimentProvider {
    pub station: Arc<StationInner>,
}

impl ExperimentProvider for StationExperimentProvider {
    fn create_experiment(
        &self,
        name: String,
        attributes: HashMap<String, Value>,
    ) -> Job<ExperimentRun, ExperimentError> {
        let station = Arc::clone(&self.station);
        self.station.attach(async move {
            create_run(station, name, attributes)
                .await
                .map_err(|error| ExperimentError {
                    kind: ExperimentErrorKind::Internal,
                    message: "Failed to start Station experiment run".to_string(),
                    source: Some(Box::new(error)),
                })
        })
    }
}

async fn create_run(
    station: Arc<StationInner>,
    name: String,
    attributes: HashMap<String, Value>,
) -> Result<ExperimentRun, RunError> {
    let experiments_client = station.client.experiments();
    let experiment = experiments_client
        .create(CreateExperimentRequest {
            name: Some(name),
            description: None,
            attributes,
        })
        .await?;

    let experiment_num = experiment.experiment_num;
    let path = ExperimentPath::new(experiment_num);
    let cancel_token = CancelToken::new();
    let control = ExperimentRunControl::new(cancel_token.clone());

    let artifact_uploader = StationArtifactUploader::new(Arc::clone(&station), path);

    let ws = experiments_client
        .create_run_websocket(experiment_num)
        .await?;

    // Both loops run on the Station's runtime; the run only ever touches their mailboxes.
    let (socket, run) = SocketHandle::start(ws, control.clone());
    station.runtime.spawn(run);
    let (session, ship) = RemoteExperimentSession::start(Box::new(artifact_uploader), socket);
    station.runtime.spawn(ship);

    let reader = StationArtifactReader::new(station);
    let id = ExperimentId::from(experiment_num.to_string());

    Ok(ExperimentRun::new_with_control(
        id, session, reader, control,
    ))
}
