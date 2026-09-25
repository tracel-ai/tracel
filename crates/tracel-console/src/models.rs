use std::sync::Arc;

use tracel_artifact::upload::{
    MultipartUploadFile, MultipartUploadPart, MultipartUploadSource, UploadError,
    upload_bundle_multipart_with_client_and_observer,
};
use tracel_artifact::{FileTransferClient, ReqwestTransferClient, TransferObserver};
use tracel_client::{
    console::model::request::{
        CreateModelRequest, ModelFileSpecRequest, RequestModelVersionUploadRequest,
    },
    console::model::response::{
        ModelDownloadResponse, ModelListResponse, ModelResponse, ModelVersionListResponse,
        ModelVersionResponse, ModelVersionStateResponse,
    },
    error::{ApiErrorCode, ClientError},
};
use tracel_models::{
    Model, ModelOps, ModelVersion, ModelsError, VersionFile, VersionFileReader, VersionFileSource,
    VersionId, VersionManifest, VersionSpec, VersionState,
};

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

impl ModelOps for ConsoleModelOps {
    fn list_models(&self) -> Result<Vec<Model>, ModelsError> {
        let response = self
            .scope
            .console
            .client
            .list_models(&self.scope.owner, &self.scope.project)
            .map_err(console_failure)?;
        Ok(models_from_wire(response))
    }

    fn get_model(&self, name: &str) -> Result<Model, ModelsError> {
        self.scope
            .console
            .client
            .get_model(&self.scope.owner, &self.scope.project, name)
            .map(model_from_wire)
            .map_err(|error| map_model_error(error, name))
    }

    fn list_versions(&self, model: &str) -> Result<Vec<ModelVersion>, ModelsError> {
        let response = self
            .scope
            .console
            .client
            .list_model_versions(&self.scope.owner, &self.scope.project, model)
            .map_err(|error| map_model_error(error, model))?;
        Ok(model_versions_from_wire(response))
    }

    fn get_version(&self, model: &str, spec: VersionSpec) -> Result<ModelVersion, ModelsError> {
        let client = &self.scope.console.client;
        let (owner, project) = (&self.scope.owner, &self.scope.project);
        let response = match &spec {
            VersionSpec::Exact(id) => {
                client.get_model_version(owner, project, model, self.route_version(model, id)?)
            }
            VersionSpec::Latest => {
                client.resolve_model_version_ref(owner, project, model, "latest")
            }
            VersionSpec::Alias(alias) => {
                client.resolve_model_version_ref(owner, project, model, alias)
            }
        };

        response
            .map(model_version_from_wire)
            .map_err(|error| map_version_error(error, model, &spec))
    }

    fn fetch_version_files(
        &self,
        model: &str,
        id: &VersionId,
    ) -> Result<Vec<Box<dyn VersionFileSource>>, ModelsError> {
        let version = self.route_version(model, id)?;
        let response = self
            .scope
            .console
            .client
            .presign_model_download(&self.scope.owner, &self.scope.project, model, version)
            .map_err(|error| map_version_error(error, model, &VersionSpec::Exact(id.clone())))?;
        Ok(file_sources_from_wire(
            &self.scope.console.transfer_client,
            response,
        ))
    }

    fn create_model(&self, name: &str, description: Option<&str>) -> Result<Model, ModelsError> {
        self.scope
            .console
            .client
            .create_model(
                &self.scope.owner,
                &self.scope.project,
                CreateModelRequest {
                    name: name.to_string(),
                    description: description.map(str::to_string),
                },
            )
            .map(model_from_wire)
            .map_err(console_failure)
    }

    fn publish_version(
        &self,
        model: &str,
        files: &[VersionFile],
        contents: &dyn MultipartUploadSource,
        metadata: Option<&serde_json::Value>,
        mut observer: &mut dyn TransferObserver,
    ) -> Result<ModelVersion, ModelsError> {
        let request = RequestModelVersionUploadRequest {
            files: files
                .iter()
                .map(|file| ModelFileSpecRequest {
                    rel_path: file.rel_path.clone(),
                    size_bytes: file.size_bytes,
                    checksum: file.checksum.clone(),
                })
                .collect(),
            metadata: metadata.cloned(),
        };
        let planned = self
            .scope
            .console
            .client
            .request_model_version_upload(&self.scope.owner, &self.scope.project, model, request)
            .map_err(|error| map_model_error(error, model))?;

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

        upload_bundle_multipart_with_client_and_observer(
            &self.scope.console.transfer_client,
            &contents,
            &uploads,
            &mut observer,
        )
        .map_err(model_upload_failure)?;

        let version = VersionSpec::Exact(VersionId::new(planned.version.to_string()));
        self.scope
            .console
            .client
            .complete_model_version_upload(
                &self.scope.owner,
                &self.scope.project,
                model,
                planned.version,
            )
            .map_err(|error| map_version_error(error, model, &version))?;

        self.scope
            .console
            .client
            .get_model_version(
                &self.scope.owner,
                &self.scope.project,
                model,
                planned.version,
            )
            .map(model_version_from_wire)
            .map_err(|error| map_version_error(error, model, &version))
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

fn model_versions_from_wire(response: ModelVersionListResponse) -> Vec<ModelVersion> {
    response
        .items
        .into_iter()
        .map(model_version_from_wire)
        .collect()
}

fn model_version_from_wire(value: ModelVersionResponse) -> ModelVersion {
    ModelVersion {
        id: VersionId::new(value.version.to_string()),
        version: Some(value.version),
        state: state_from_wire(value.state),
        failure_reason: value.failure_reason,
        size_bytes: value.size,
        checksum: value.digest,
        aliases: value.aliases,
        published_by: Some(value.created_by.username),
        created_at: console_timestamp(&value.created_at),
        manifest: VersionManifest {
            files: value
                .manifest
                .files
                .into_iter()
                .map(|file| VersionFile {
                    rel_path: file.rel_path,
                    size_bytes: file.size_bytes,
                    checksum: file.checksum,
                })
                .collect(),
        },
        metadata: value.metadata,
        deleted_at: value.deleted_at.as_deref().and_then(console_timestamp),
    }
}

fn state_from_wire(state: ModelVersionStateResponse) -> VersionState {
    match state {
        ModelVersionStateResponse::Pending => VersionState::Pending,
        ModelVersionStateResponse::Ready => VersionState::Ready,
        ModelVersionStateResponse::Failed => VersionState::Failed,
        ModelVersionStateResponse::Deleted => VersionState::Deleted,
    }
}

fn file_sources_from_wire(
    transfer_client: &ReqwestTransferClient,
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
                transfer_client: transfer_client.clone(),
            }) as Box<dyn VersionFileSource>
        })
        .collect()
}

struct ConsoleVersionFileSource {
    file: VersionFile,
    url: String,
    transfer_client: ReqwestTransferClient,
}

impl VersionFileSource for ConsoleVersionFileSource {
    fn file(&self) -> &VersionFile {
        &self.file
    }

    fn open(&self, _canonical_path: &str) -> Result<VersionFileReader, ModelsError> {
        self.transfer_client
            .get_reader(&self.url, Some(self.file.size_bytes))
            .map_err(|error| ModelsError::Transport(error.to_string()))
    }
}

fn map_model_error(error: ClientError, name: &str) -> ModelsError {
    if let Some(refusal) = refusal(&error, name, None) {
        return refusal;
    }
    if client_error_is_not_found(&error) {
        return ModelsError::ModelNotFound {
            name: name.to_string(),
        };
    }
    console_failure(error)
}

fn map_version_error(error: ClientError, model: &str, version: &VersionSpec) -> ModelsError {
    if let Some(refusal) = refusal(&error, model, Some(version)) {
        return refusal;
    }
    if client_error_is_not_found(&error) {
        return ModelsError::VersionNotFound {
            model: model.to_string(),
            version: version.clone(),
        };
    }
    console_failure(error)
}

fn refusal(error: &ClientError, model: &str, version: Option<&VersionSpec>) -> Option<ModelsError> {
    let model = model.to_string();
    let refusal = match (error.code()?, version) {
        (ApiErrorCode::Model, _) => ModelsError::ModelNotFound { name: model },
        (ApiErrorCode::ModelAlias, Some(VersionSpec::Alias(alias))) => ModelsError::AliasNotFound {
            model,
            alias: alias.clone(),
        },
        (ApiErrorCode::ModelVersion, Some(version)) => ModelsError::VersionNotFound {
            model,
            version: version.clone(),
        },
        (ApiErrorCode::ModelVersionNotReady, Some(version)) => ModelsError::VersionNotReady {
            model,
            version: version.clone(),
        },
        (ApiErrorCode::ModelVersionDeleted, Some(version)) => ModelsError::VersionDeleted {
            model,
            version: version.clone(),
        },
        (code, _) if error.is_conflict() => ModelsError::Conflict {
            model,
            code: code.to_string(),
        },
        _ => return None,
    };
    Some(refusal)
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
