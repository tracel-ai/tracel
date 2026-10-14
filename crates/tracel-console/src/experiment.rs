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
    ArtifactKind, CancelToken, ExperimentId, ExperimentLocation, ExperimentProvider, ExperimentRun,
    ExperimentRunControl,
};
use tracel_experiment_remote::{ArtifactUploadError, ArtifactUploader, RemoteExperimentSession};
use url::Url;

use crate::console::ProjectScope;
use crate::{Namespace, NamespaceKind};

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
enum StartRunError {
    Http(#[from] ClientError),
    WebSocket(#[from] WebSocketError),
}

fn create_run(
    scope: &Arc<ProjectScope>,
    name: String,
    attributes: HashMap<String, Value>,
) -> Result<ExperimentRun, StartRunError> {
    let experiment = scope.client()?.create_experiment(
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

    let ws = scope.client()?.create_experiment_run_websocket(
        &scope.owner,
        &scope.project,
        experiment_num,
    )?;

    let session = RemoteExperimentSession::new(Box::new(artifact_uploader), ws, control.clone());

    let reader = ConsoleArtifactReader::new(Arc::clone(scope));
    let id = ExperimentId::from(format!("{experiment_num}"));
    let run = ExperimentRun::new_with_control(id, session, reader, control);

    Ok(match page_of(scope, experiment_num) {
        Some(page) => run.with_location(ExperimentLocation::Url(page.into())),
        None => run,
    })
}

/// The console page of experiment `num` of the project `scope` names, when the project's owner
/// can be read.
fn page_of(scope: &ProjectScope, num: i32) -> Option<Url> {
    let kind = match scope.owner_kind() {
        Ok(kind) => kind,
        Err(error) => {
            tracing::warn!("Could not find the console page of experiment {num}: {error}");
            return None;
        }
    };
    let owner = Namespace {
        name: scope.owner.clone(),
        kind,
    };
    experiment_page(scope.console().base_url(), &owner, &scope.project, num)
}

/// The page of experiment `num` of `project`, owned by `owner`, on the console whose API is at
/// `api_url`.
///
/// The console serves its API under `api/` of its web address, and an experiment's page at
/// `users/<namespace>/projects/<project>/experiments/<num>` of it, or under `orgs/` for a project
/// an organization owns. `None` when the API is not served under `api/`.
fn experiment_page(api_url: &Url, owner: &Namespace, project: &str, num: i32) -> Option<Url> {
    let mut page = Url::parse(api_url.as_str().strip_suffix("api/")?).ok()?;
    let owners = match owner.kind {
        NamespaceKind::User => "users",
        NamespaceKind::Organization => "orgs",
    };
    page.path_segments_mut().ok()?.pop_if_empty().extend([
        owners,
        &owner.name,
        "projects",
        project,
        "experiments",
        &num.to_string(),
    ]);
    Some(page)
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

        let res = self.scope.client()?.create_artifact(
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

        self.scope.client()?.complete_artifact_upload(
            &self.scope.owner,
            &self.scope.project,
            self.experiment_num,
            &res.id,
            None,
        )?;

        Ok(res.id)
    }

    /// Downloads the files of the artifact `artifact_id` into a temporary bundle.
    fn download(&self, artifact_id: &str) -> Result<FsBundle, ArtifactError> {
        let resp = self.scope.client()?.presign_artifact_download(
            &self.scope.owner,
            &self.scope.project,
            self.experiment_num,
            artifact_id,
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

    /// The artifact of the experiment named `name`.
    fn fetch(&self, name: &str) -> Result<ArtifactResponse, ArtifactError> {
        let listed = self.scope.client()?.list_artifacts_by_name(
            &self.scope.owner,
            &self.scope.project,
            self.experiment_num,
            name,
        )?;
        named(listed.items, name, |artifact| &artifact.name)
    }
}

/// The one item of `items` whose name, which `name_of` gives, is `name`.
///
/// The console's name filter lists the artifacts whose names contain the name asked for, so
/// asking for `model` can list `model-2` as well.
fn named<T>(items: Vec<T>, name: &str, name_of: impl Fn(&T) -> &str) -> Result<T, ArtifactError> {
    let mut matching = items.into_iter().filter(|item| name_of(item) == name);
    let found = matching
        .next()
        .ok_or_else(|| ArtifactError::NotFound(name.to_owned()))?;
    match matching.count() {
        0 => Ok(found),
        others => Err(ArtifactError::Ambiguous {
            name: name.to_owned(),
            count: others + 1,
        }),
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
    #[error("Artifact name is ambiguous: {count} artifacts are named {name}")]
    Ambiguous { name: String, count: usize },
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
            .download(&artifact.id)
            .map_err(|err| {
                ExperimentReaderError::with_source("Failed to download experiment artifact", err)
            })
            .map(|bundle| {
                LoadedArtifact::new(
                    ArtifactRef {
                        id: artifact.id,
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

#[cfg(test)]
mod tests {
    use tracel_client::console::Env;

    use super::*;

    /// The id of the artifact `named` picks among `(id, name)` pairs.
    fn pick(
        artifacts: &[(&'static str, &'static str)],
        name: &str,
    ) -> Result<&'static str, ArtifactError> {
        named(artifacts.to_vec(), name, |(_, name)| name).map(|(id, _)| id)
    }

    #[test]
    fn an_artifact_is_picked_by_its_exact_name() {
        let listed = [("2", "model-2"), ("1", "model"), ("3", "model.bpk")];

        assert_eq!(pick(&listed, "model").unwrap(), "1");
        assert_eq!(pick(&listed, "model-2").unwrap(), "2");
    }

    #[test]
    fn a_name_no_artifact_has_exactly_is_not_found() {
        let error = pick(&[("2", "model-2")], "model").unwrap_err();

        assert!(matches!(&error, ArtifactError::NotFound(name) if name == "model"));
        assert!(matches!(
            pick(&[], "model"),
            Err(ArtifactError::NotFound(_))
        ));
    }

    #[test]
    fn a_name_several_artifacts_have_is_ambiguous() {
        let error = pick(&[("1", "model"), ("2", "model-2"), ("3", "model")], "model").unwrap_err();

        assert!(matches!(&error, ArtifactError::Ambiguous { count: 2, .. }));
        assert_eq!(
            error.to_string(),
            "Artifact name is ambiguous: 2 artifacts are named model"
        );
    }

    fn page(api_url: &str, owner: Namespace, project: &str, num: i32) -> Option<String> {
        experiment_page(&Url::parse(api_url).unwrap(), &owner, project, num).map(String::from)
    }

    #[test]
    fn a_user_project_experiment_is_under_users() {
        assert_eq!(
            page(
                Env::Production.get_url().as_str(),
                Namespace::user("alice"),
                "mnist",
                42
            )
            .as_deref(),
            Some("https://console.tracel.ai/users/alice/projects/mnist/experiments/42")
        );
    }

    #[test]
    fn an_organization_project_experiment_is_under_orgs() {
        assert_eq!(
            page(
                "https://console.example.com/api/",
                Namespace::organization("tracel"),
                "vision",
                7
            )
            .as_deref(),
            Some("https://console.example.com/orgs/tracel/projects/vision/experiments/7")
        );
    }

    #[test]
    fn names_are_escaped_as_path_segments() {
        assert_eq!(
            page(
                "https://console.example.com/api/",
                Namespace::user("a b"),
                "x/y",
                1
            )
            .as_deref(),
            Some("https://console.example.com/users/a%20b/projects/x%2Fy/experiments/1")
        );
    }

    #[test]
    fn an_api_not_served_under_api_has_no_pages() {
        assert_eq!(
            page(
                Env::Development.get_url().as_str(),
                Namespace::user("alice"),
                "mnist",
                1
            ),
            None
        );
    }
}
