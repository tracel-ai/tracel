use std::sync::Arc;

use bytes::Bytes;
use serde::Deserialize;
use tracel_artifact::upload::{
    MultipartUploadFile, MultipartUploadPart, MultipartUploadSource, UploadError, upload_multipart,
};
use tracel_artifact::{HttpTransferClient, TransferClient, TransferError, TransferObserver};
use tracel_client::console::Client;
use tracel_client::{
    console::model::request::{
        CreateModelRequest, ModelFileSpecRequest, RequestModelVersionUploadRequest,
    },
    console::model::response::{
        ModelDownloadResponse, ModelListResponse, ModelResponse, ModelVersionListResponse,
        ModelVersionResponse,
    },
    error::ClientError,
};
use tracel_models::{
    Model, ModelOps, ModelVersion, ModelsError, VersionFile, VersionFileSource, VersionId,
    VersionManifest, VersionSpec,
};
use tracel_task::{Spawn, Streaming, Task};

use crate::ConsoleError;
use crate::console::ProjectScope;
use crate::error::client_error_is_not_found;
use crate::wire::console_timestamp;

#[derive(Clone)]
pub struct ConsoleModelOps {
    pub scope: Arc<ProjectScope>,
}

impl ConsoleModelOps {
    fn route_version(&self, model: &str, id: &VersionId) -> Result<u32, ModelsError> {
        id.as_str()
            .parse()
            .map_err(|_| ModelsError::VersionNotFound {
                model: model.to_string(),
                version: VersionSpec::Exact(id.clone()),
            })
    }
}

impl ConsoleModelOps {
    fn location(&self) -> (String, String) {
        (self.scope.owner.clone(), self.scope.project.clone())
    }

    /// Runs one client call on the backend's executor. Until `tracel-client` is asynchronous
    /// every call blocks, so it goes to the blocking lane rather than the scheduler.
    fn call<T, E, F>(&self, call: F) -> Task<T, E>
    where
        F: FnOnce(&Client) -> Result<T, E> + Send + 'static,
        T: Send + 'static,
        E: Send + 'static,
    {
        let client = self.scope.console.client.clone();
        Task::spawn_blocking(&*self.scope.console.spawn, move || call(&client))
    }
}

impl ModelOps for ConsoleModelOps {
    fn list_models(&self) -> Task<Vec<Model>, ModelsError> {
        let (owner, project) = self.location();
        self.call(move |client| {
            client
                .list_models(&owner, &project)
                .map(models_from_wire)
                .map_err(console_failure)
        })
    }

    fn get_model(&self, name: String) -> Task<Model, ModelsError> {
        let (owner, project) = self.location();
        self.call(move |client| {
            client
                .get_model(&owner, &project, &name)
                .map(model_from_wire)
                .map_err(|error| map_model_error(error, &name))
        })
    }

    fn list_versions(&self, model: String) -> Task<Vec<ModelVersion>, ModelsError> {
        let (owner, project) = self.location();
        self.call(move |client| {
            client
                .list_model_versions(&owner, &project, &model)
                .map_err(|error| map_model_error(error, &model))
                .and_then(model_versions_from_wire)
        })
    }

    fn get_version(&self, model: String, spec: VersionSpec) -> Task<ModelVersion, ModelsError> {
        let this = self.clone();
        Task::spawn(&*self.scope.console.spawn, async move {
            let id = match &spec {
                VersionSpec::Exact(id) => id.clone(),
                VersionSpec::Latest => this
                    .list_versions(model.clone())
                    .await?
                    .into_iter()
                    .max_by_key(|version| version.version)
                    .map(|version| version.id)
                    .ok_or_else(|| ModelsError::VersionNotFound {
                        model: model.clone(),
                        version: spec.clone(),
                    })?,
            };

            let route = this.route_version(&model, &id)?;
            let (owner, project) = this.location();
            this.call(move |client| {
                client
                    .get_model_version(&owner, &project, &model, route)
                    .map_err(|error| map_version_error(error, &model, &id))
                    .and_then(model_version_from_wire)
            })
            .await
        })
    }

    fn fetch_version_files(
        &self,
        model: String,
        id: VersionId,
    ) -> Task<Vec<Box<dyn VersionFileSource>>, ModelsError> {
        let route = match self.route_version(&model, &id) {
            Ok(route) => route,
            Err(error) => return Task::failed(error),
        };
        let (owner, project) = self.location();
        let transfer = self.scope.console.transfer.clone();
        let spawn = Arc::clone(&self.scope.console.spawn);
        self.call(move |client| {
            client
                .presign_model_download(&owner, &project, &model, route)
                .map_err(|error| map_version_error(error, &model, &id))
                .map(|response| file_sources_from_wire(&transfer, &spawn, response))
        })
    }

    fn create_model(&self, name: String, description: Option<String>) -> Task<Model, ModelsError> {
        let (owner, project) = self.location();
        self.call(move |client| {
            client
                .create_model(&owner, &project, CreateModelRequest { name, description })
                .map(model_from_wire)
                .map_err(console_failure)
        })
    }

    fn publish_version(
        &self,
        model: String,
        files: Vec<VersionFile>,
        contents: Arc<dyn MultipartUploadSource>,
        metadata: Option<serde_json::Value>,
        mut observer: Box<dyn TransferObserver>,
    ) -> Task<ModelVersion, ModelsError> {
        let this = self.clone();
        Task::spawn(&*self.scope.console.spawn, async move {
            let (owner, project) = this.location();
            let request = RequestModelVersionUploadRequest {
                files: files
                    .iter()
                    .map(|file| ModelFileSpecRequest {
                        rel_path: file.rel_path.clone(),
                        size_bytes: file.size_bytes,
                        checksum: file.checksum.clone(),
                    })
                    .collect(),
                metadata,
            };
            let planned = {
                let (owner, project, model) = (owner.clone(), project.clone(), model.clone());
                this.call(move |client| {
                    client
                        .request_model_version_upload(&owner, &project, &model, request)
                        .map_err(|error| map_model_error(error, &model))
                })
                .await?
            };

            let uploads = planned
                .files
                .into_iter()
                .map(|file| MultipartUploadFile {
                    rel_path: file.rel_path,
                    parts: file
                        .urls
                        .parts
                        .into_iter()
                        .map(|part| MultipartUploadPart {
                            part: part.part,
                            url: part.url,
                            size_bytes: part.size_bytes,
                        })
                        .collect(),
                })
                .collect::<Vec<_>>();

            upload_multipart(
                &this.scope.console.transfer,
                &*contents,
                &uploads,
                &mut *observer,
            )
            .await
            .map_err(model_upload_failure)?;

            let version = planned.version;
            {
                let (owner, project, model) = (owner.clone(), project.clone(), model.clone());
                this.call(move |client| {
                    client
                        .complete_model_version_upload(&owner, &project, &model, version)
                        .map_err(|error| map_model_error(error, &model))
                })
                .await?;
            }

            this.call(move |client| {
                client
                    .get_model_version(&owner, &project, &model, version)
                    .map_err(|error| map_model_error(error, &model))
                    .and_then(model_version_from_wire)
            })
            .await
        })
    }
}

fn models_from_wire(response: ModelListResponse) -> Vec<Model> {
    response.items.into_iter().map(model_from_wire).collect()
}

fn model_from_wire(value: ModelResponse) -> Model {
    Model {
        id: value.id,
        name: value.name,
        description: value.description,
        published_by: Some(value.created_by.username),
        created_at: console_timestamp(&value.created_at),
        version_count: value.version_count,
        latest_version: value.latest_version,
    }
}

fn model_versions_from_wire(
    response: ModelVersionListResponse,
) -> Result<Vec<ModelVersion>, ModelsError> {
    response
        .items
        .into_iter()
        .map(model_version_from_wire)
        .collect()
}

fn model_version_from_wire(value: ModelVersionResponse) -> Result<ModelVersion, ModelsError> {
    let manifest: WireManifest = serde_json::from_value(value.manifest)
        .map_err(|error| ModelsError::other(ConsoleError::InvalidResponse(error.to_string())))?;

    Ok(ModelVersion {
        id: VersionId::new(value.version.to_string()),
        version: Some(value.version),
        size_bytes: value.size,
        checksum: value.checksum,
        published_by: Some(value.created_by.username),
        created_at: console_timestamp(&value.created_at),
        manifest: manifest.into(),
        metadata: value.metadata,
    })
}

/// The manifest as this console writes it, so the model domain never has to name a field the
/// way one backend happens to spell it.
#[derive(Deserialize)]
struct WireManifest {
    files: Vec<WireManifestFile>,
}

#[derive(Deserialize)]
struct WireManifestFile {
    rel_path: String,
    size_bytes: u64,
    checksum: String,
}

impl From<WireManifest> for VersionManifest {
    fn from(value: WireManifest) -> Self {
        VersionManifest {
            files: value
                .files
                .into_iter()
                .map(|file| VersionFile {
                    rel_path: file.rel_path,
                    size_bytes: file.size_bytes,
                    checksum: file.checksum,
                })
                .collect(),
        }
    }
}

fn file_sources_from_wire(
    transfer: &HttpTransferClient,
    spawn: &Arc<dyn Spawn>,
    response: ModelDownloadResponse,
) -> Vec<Box<dyn VersionFileSource>> {
    response
        .files
        .into_iter()
        .map(|file| {
            Box::new(ConsoleVersionFileSource {
                file: VersionFile {
                    rel_path: file.rel_path,
                    size_bytes: file.size_bytes,
                    checksum: file.checksum,
                },
                url: file.url,
                transfer: transfer.clone(),
                spawn: Arc::clone(spawn),
            }) as Box<dyn VersionFileSource>
        })
        .collect()
}

struct ConsoleVersionFileSource {
    file: VersionFile,
    url: String,
    transfer: HttpTransferClient,
    spawn: Arc<dyn Spawn>,
}

impl VersionFileSource for ConsoleVersionFileSource {
    fn file(&self) -> &VersionFile {
        &self.file
    }

    fn open(&self, _canonical_path: String) -> Streaming<Bytes, TransferError> {
        let transfer = self.transfer.clone();
        let url = self.url.clone();
        let size = self.file.size_bytes;
        Streaming::spawn(&*self.spawn, 1, |sink| async move {
            match transfer.get(&url, Some(size)).await {
                Ok(body) => sink.forward(body).await,
                Err(error) => sink.fail(error).await,
            }
        })
    }
}

fn map_model_error(error: ClientError, name: &str) -> ModelsError {
    if client_error_is_not_found(&error) {
        return ModelsError::ModelNotFound {
            name: name.to_string(),
        };
    }
    console_failure(error)
}

fn map_version_error(error: ClientError, model: &str, id: &VersionId) -> ModelsError {
    if client_error_is_not_found(&error) {
        return ModelsError::VersionNotFound {
            model: model.to_string(),
            version: VersionSpec::Exact(id.clone()),
        };
    }
    console_failure(error)
}

fn console_failure(error: ClientError) -> ModelsError {
    match ConsoleError::from(error) {
        ConsoleError::Transport(reason) => ModelsError::Transport(reason),
        error => ModelsError::other(error),
    }
}

fn model_upload_failure(error: UploadError) -> ModelsError {
    match error {
        UploadError::Cancelled { .. } => ModelsError::Cancelled,
        error @ UploadError::Transfer { .. } => ModelsError::Transport(error.to_string()),
        error => ModelsError::other(error),
    }
}
