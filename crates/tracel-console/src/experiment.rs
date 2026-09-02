//! Runs experiments against the console: an HTTP-created experiment record paired with a
//! websocket session for events, with artifacts moved over separate REST calls.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use serde_json::Value;
use tracel_artifact::bundle::FsBundle;
use tracel_artifact::download::{ArtifactDownloadFile, DownloadError, download_artifacts_to_sink};
use tracel_artifact::upload::{
    MultipartUploadFile, MultipartUploadPart, UploadError, upload_bundle_multipart,
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
use tracel_experiment_remote::{ArtifactUploadError, ArtifactUploader, RemoteExperimentSession};

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
    ) -> Result<ExperimentRun, ExperimentError> {
        create_run(&self.scope, name, attributes).map_err(|e| ExperimentError {
            kind: ExperimentErrorKind::Internal,
            message: "Failed to start console experiment run".to_string(),
            source: Some(Box::new(e)),
        })
    }
}

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
enum CloudError {
    Http(#[from] ClientError),
    WebSocket(#[from] WebSocketError),
}

fn create_run(
    scope: &Arc<ProjectScope>,
    name: String,
    attributes: HashMap<String, Value>,
) -> Result<ExperimentRun, CloudError> {
    let experiment = scope.console.client.create_experiment(
        &scope.owner,
        &scope.project,
        Some(name),
        None,
        attributes,
    )?;

    let experiment_num = experiment.experiment_num;
    let cancel_token = CancelToken::new();
    let control = ExperimentRunControl::new(cancel_token.clone());

    let artifact_uploader = ConsoleArtifactUploader::new(Arc::clone(scope), experiment_num);

    let ws = scope.console.client.create_experiment_run_websocket(
        &scope.owner,
        &scope.project,
        experiment_num,
    )?;

    let session = RemoteExperimentSession::new(Box::new(artifact_uploader), ws, control.clone());

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
    fn upload(
        &self,
        name: impl Into<String>,
        kind: ArtifactKind,
        bundle: &FsBundle,
    ) -> Result<String, ArtifactError> {
        let name = name.into();

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

        let res = self.scope.console.client.create_artifact(
            &self.scope.owner,
            &self.scope.project,
            self.experiment_num,
            CreateArtifactRequest {
                name: name.clone(),
                kind: artifact_kind_name(kind).to_string(),
                files: specs,
            },
        )?;

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
        upload_bundle_multipart(bundle, &uploads)?;

        self.scope.console.client.complete_artifact_upload(
            &self.scope.owner,
            &self.scope.project,
            self.experiment_num,
            &res.id,
            None,
        )?;

        Ok(res.id)
    }

    fn download(&self, name: impl AsRef<str>) -> Result<FsBundle, ArtifactError> {
        let name = name.as_ref();
        let artifact = self.fetch(name)?;
        let resp = self.scope.console.client.presign_artifact_download(
            &self.scope.owner,
            &self.scope.project,
            self.experiment_num,
            &artifact.id.to_string(),
        )?;

        let mut files = Vec::with_capacity(resp.files.len());
        for file in resp.files {
            files.push(ArtifactDownloadFile {
                rel_path: file.rel_path,
                url: file.url,
                size_bytes: None,
                checksum: None,
            });
        }

        let mut bundle = FsBundle::temp()
            .map_err(|e| ArtifactError::Internal(format!("Failed to create temp bundle: {e}")))?;

        download_artifacts_to_sink(&mut bundle, &files)?;

        Ok(bundle)
    }

    fn fetch(&self, name: impl AsRef<str>) -> Result<ArtifactResponse, ArtifactError> {
        let name = name.as_ref();
        self.scope
            .console
            .client
            .list_artifacts_by_name(
                &self.scope.owner,
                &self.scope.project,
                self.experiment_num,
                name,
            )?
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
        name: &str,
    ) -> Result<LoadedArtifact, ExperimentReaderError> {
        let num = experiment_id
            .parse::<i32>()
            .ok_or_else(|| ExperimentReaderError::new("Invalid experiment ID format"))?;

        let client = ExperimentArtifactClient {
            scope: Arc::clone(&self.scope),
            experiment_num: num,
        };
        let artifact = client.fetch(name).map_err(|err| {
            ExperimentReaderError::with_source("Failed to resolve experiment artifact", err)
        })?;

        client
            .download(name)
            .map_err(|err| {
                ExperimentReaderError::with_source("Failed to download experiment artifact", err)
            })
            .map(|bundle| {
                LoadedArtifact::new(
                    ArtifactRef {
                        id: artifact.id.to_string(),
                        name: name.to_string(),
                    },
                    bundle,
                )
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
        name: &str,
        kind: ArtifactKind,
        bundle: &FsBundle,
    ) -> Result<(), ArtifactUploadError> {
        self.client
            .upload(name, kind, bundle)
            .map(|_| ())
            .map_err(|e| ArtifactUploadError {
                message: format!("Failed to upload artifact '{name}'"),
                source: Some(Box::new(e)),
            })
    }
}
