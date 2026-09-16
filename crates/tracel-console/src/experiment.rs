//! Runs experiments against the console: an HTTP-created experiment record paired with a
//! websocket session for events, with artifacts moved over separate REST calls.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use serde_json::Value;
use tracel_artifact::bundle::FsBundle;
use tracel_artifact::download::{ArtifactDownloadFile, DownloadError, download_into};
use tracel_artifact::upload::{
    MultipartUploadFile, MultipartUploadPart, UploadError, upload_multipart,
};
use tracel_client::ClientError;
use tracel_client::console::artifact::{
    request::{ArtifactFileSpecRequest, CreateArtifactRequest},
    response::ArtifactResponse,
};
use tracel_client::websocket::WebSocketError;
use tracel_experiment::error::{ExperimentError, ExperimentErrorKind};
use tracel_experiment::reader::{
    ArtifactRef, ExperimentArtifactReader, ExperimentReaderError, LoadedArtifact,
};
use tracel_experiment::{
    ArtifactKind, CancelToken, ExperimentId, ExperimentProvider, ExperimentRun,
    ExperimentRunControl,
};
use tracel_experiment_remote::{
    ArtifactUploadError, ArtifactUploader, RemoteExperimentSession, SocketHandle,
};
use tracel_task::Job;

use crate::console::ProjectScope;

/// Experiment provider backed by the console's experiment run protocol.
pub struct ConsoleExperimentProvider {
    scope: Arc<ProjectScope>,
}

impl ConsoleExperimentProvider {
    pub fn new(scope: Arc<ProjectScope>) -> Self {
        Self { scope }
    }
}

impl ExperimentProvider for ConsoleExperimentProvider {
    fn create_experiment(
        &self,
        name: String,
        attributes: HashMap<String, Value>,
    ) -> Job<ExperimentRun, ExperimentError> {
        let scope = Arc::clone(&self.scope);
        self.scope.console.attach(async move {
            create_run(&scope, name, attributes)
                .await
                .map_err(|e| ExperimentError {
                    kind: ExperimentErrorKind::Internal,
                    message: "Failed to start console experiment run".to_string(),
                    source: Some(Box::new(e)),
                })
        })
    }
}

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
enum CloudError {
    Http(#[from] ClientError),
    WebSocket(#[from] WebSocketError),
}

async fn create_run(
    scope: &Arc<ProjectScope>,
    name: String,
    attributes: HashMap<String, Value>,
) -> Result<ExperimentRun, CloudError> {
    let console = &scope.console;
    let experiment = console
        .client
        .create_experiment(&scope.owner, &scope.project, Some(name), None, attributes)
        .await?;

    let experiment_num = experiment.experiment_num;
    let cancel_token = CancelToken::new();
    let control = ExperimentRunControl::new(cancel_token.clone());

    let artifact_uploader = ConsoleArtifactUploader::new(Arc::clone(scope), experiment_num);

    let ws = console
        .client
        .create_experiment_run_websocket(&scope.owner, &scope.project, experiment_num)
        .await?;

    // Both loops run on the connection's runtime; the run only ever touches their mailboxes.
    let (socket, run) = SocketHandle::start(ws, control.clone());
    console.runtime.spawn(run);
    let (session, ship) = RemoteExperimentSession::start(Box::new(artifact_uploader), socket);
    console.runtime.spawn(ship);

    let reader = ConsoleArtifactReader::new(Arc::clone(scope));
    let id = ExperimentId::from(format!("{experiment_num}"));

    Ok(ExperimentRun::new_with_control(
        id, session, reader, control,
    ))
}

/// A scope for artifact operations within a specific experiment.
#[derive(Clone)]
struct ExperimentArtifactClient {
    scope: Arc<ProjectScope>,
    experiment_num: i32,
}

impl ExperimentArtifactClient {
    async fn upload(
        &self,
        name: String,
        kind: ArtifactKind,
        bundle: FsBundle,
    ) -> Result<String, ArtifactError> {
        let mut specs = Vec::with_capacity(bundle.files().len());
        for f in bundle.files() {
            let size_bytes = f.size_bytes.ok_or_else(|| {
                ArtifactError::Internal(format!("Missing file size for {}", f.rel_path))
            })?;
            let checksum = f.checksum.clone().ok_or_else(|| {
                ArtifactError::Internal(format!("Missing checksum for {}", f.rel_path))
            })?;
            specs.push(ArtifactFileSpecRequest {
                rel_path: f.rel_path.clone(),
                size_bytes,
                checksum,
            });
        }

        let console = &self.scope.console;
        let res = console
            .client
            .create_artifact(
                &self.scope.owner,
                &self.scope.project,
                self.experiment_num,
                CreateArtifactRequest {
                    name,
                    kind: artifact_kind_name(kind).to_string(),
                    files: specs,
                },
            )
            .await?;

        let mut multipart_map = BTreeMap::new();
        for f in &res.files {
            multipart_map.insert(f.rel_path.clone(), &f.urls);
        }

        let mut uploads = Vec::with_capacity(bundle.files().len());
        for f in bundle.files() {
            let multipart_info = multipart_map.get(&f.rel_path).ok_or_else(|| {
                ArtifactError::Internal(format!(
                    "Missing multipart upload info for file {}",
                    f.rel_path
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
                rel_path: f.rel_path.clone(),
                parts,
            });
        }
        upload_multipart(&console.transfer, &bundle, &uploads, &mut ()).await?;

        console
            .client
            .complete_artifact_upload(
                &self.scope.owner,
                &self.scope.project,
                self.experiment_num,
                &res.id,
                None,
            )
            .await?;

        Ok(res.id)
    }

    async fn download(&self, name: &str) -> Result<(ArtifactResponse, FsBundle), ArtifactError> {
        let artifact = self.fetch(name).await?;
        let console = &self.scope.console;
        let resp = console
            .client
            .presign_artifact_download(
                &self.scope.owner,
                &self.scope.project,
                self.experiment_num,
                &artifact.id.to_string(),
            )
            .await?;

        let files: Vec<_> = resp
            .files
            .into_iter()
            .map(|file| ArtifactDownloadFile {
                rel_path: file.rel_path,
                url: file.url,
                size_bytes: None,
                checksum: None,
            })
            .collect();

        let mut bundle = FsBundle::temp()
            .map_err(|e| ArtifactError::Internal(format!("Failed to create temp bundle: {e}")))?;
        download_into(&console.transfer, &mut bundle, &files, &mut ()).await?;

        Ok((artifact, bundle))
    }

    async fn fetch(&self, name: &str) -> Result<ArtifactResponse, ArtifactError> {
        self.scope
            .console
            .client
            .list_artifacts_by_name(
                &self.scope.owner,
                &self.scope.project,
                self.experiment_num,
                name,
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
enum ArtifactError {
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

struct ConsoleArtifactReader {
    scope: Arc<ProjectScope>,
}

impl ConsoleArtifactReader {
    fn new(scope: Arc<ProjectScope>) -> Self {
        Self { scope }
    }
}

impl ExperimentArtifactReader for ConsoleArtifactReader {
    fn load_artifact_raw(
        &self,
        experiment_id: ExperimentId,
        name: String,
    ) -> Job<LoadedArtifact, ExperimentReaderError> {
        let Some(num) = experiment_id.parse::<i32>() else {
            return Job::failed(ExperimentReaderError::new("Invalid experiment ID format"));
        };

        let client = ExperimentArtifactClient {
            scope: Arc::clone(&self.scope),
            experiment_num: num,
        };
        self.scope.console.attach(async move {
            let (artifact, bundle) = client.download(&name).await.map_err(|err| {
                ExperimentReaderError::with_source("Failed to download experiment artifact", err)
            })?;

            Ok(LoadedArtifact::new(
                ArtifactRef {
                    id: artifact.id.to_string(),
                    name,
                },
                bundle,
            ))
        })
    }
}

struct ConsoleArtifactUploader {
    client: ExperimentArtifactClient,
}

impl ConsoleArtifactUploader {
    fn new(scope: Arc<ProjectScope>, experiment_num: i32) -> Self {
        Self {
            client: ExperimentArtifactClient {
                scope,
                experiment_num,
            },
        }
    }
}

impl ArtifactUploader for ConsoleArtifactUploader {
    fn upload(
        &self,
        name: String,
        kind: ArtifactKind,
        bundle: FsBundle,
    ) -> Job<(), ArtifactUploadError> {
        let client = self.client.clone();
        self.client.scope.console.attach(async move {
            client
                .upload(name.clone(), kind, bundle)
                .await
                .map(drop)
                .map_err(|e| ArtifactUploadError {
                    message: format!("Failed to upload artifact '{name}'"),
                    source: Some(Box::new(e)),
                })
        })
    }
}
