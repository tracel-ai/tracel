use std::sync::Arc;

use tracel_artifact::upload::{
    MultipartUploadFile, MultipartUploadPart, MultipartUploadSource, UploadError,
    upload_bundle_multipart_with_client_and_observer,
};
use tracel_artifact::{FileTransferClient, ReqwestTransferClient, TransferObserver};
use tracel_client::station::model::request::{
    CreateModelRequest, UploadModelFileSpecRequest, UploadModelVersionRequest,
};
use tracel_client::station::model::response::{
    ModelDownloadResponse, ModelListResponse, ModelResponse, ModelVersionListResponse,
    ModelVersionResponse, ModelVersionStateResponse,
};
use tracel_client::{ApiErrorCode, ClientError};
use tracel_models::{
    Model, ModelOps, ModelVersion, ModelsError, VersionFile, VersionFileReader, VersionFileSource,
    VersionId, VersionManifest, VersionSpec, VersionState,
};

use crate::StationError;
use crate::station::StationInner;
use crate::wire::station_timestamp;

pub struct StationModelOps {
    pub station: Arc<StationInner>,
}

impl StationModelOps {
    fn route_version(&self, model: &str, id: &VersionId) -> Result<u32, ModelsError> {
        id.as_str()
            .parse()
            .map_err(|_| ModelsError::VersionNotFound {
                model: model.to_string(),
                version: VersionSpec::Exact(id.clone()),
            })
    }
}

impl ModelOps for StationModelOps {
    fn list_models(&self) -> Result<Vec<Model>, ModelsError> {
        let response = self
            .station
            .client
            .models()
            .list()
            .map_err(station_failure)?;
        Ok(models_from_wire(response))
    }

    fn get_model(&self, name: &str) -> Result<Model, ModelsError> {
        self.station
            .client
            .models()
            .get(name)
            .map(model_from_wire)
            .map_err(|error| map_model_error(error, name))
    }

    fn list_versions(&self, model: &str) -> Result<Vec<ModelVersion>, ModelsError> {
        let response = self
            .station
            .client
            .models()
            .versions(model)
            .map_err(|error| map_model_error(error, model))?;
        Ok(model_versions_from_wire(response))
    }

    fn get_version(&self, model: &str, spec: VersionSpec) -> Result<ModelVersion, ModelsError> {
        let models = self.station.client.models();
        let response = match &spec {
            VersionSpec::Exact(id) => models
                .version(model, self.route_version(model, id)?)
                .map_err(|error| map_version_error(error, model, &spec)),
            VersionSpec::Latest => models
                .resolve(model, "latest")
                .map_err(|error| map_error(error, model, Some(&spec))),
            VersionSpec::Alias(alias) => models
                .resolve(model, alias)
                .map_err(|error| map_error(error, model, Some(&spec))),
        };

        response.map(model_version_from_wire)
    }

    fn fetch_version_files(
        &self,
        model: &str,
        id: &VersionId,
    ) -> Result<Vec<Box<dyn VersionFileSource>>, ModelsError> {
        let route = self.route_version(model, id)?;
        let response = self
            .station
            .client
            .models()
            .download(model, route)
            .map_err(|error| map_version_error(error, model, &VersionSpec::Exact(id.clone())))?;
        Ok(file_sources_from_wire(
            &self.station.transfer_client,
            response,
        ))
    }

    fn create_model(&self, name: &str, description: Option<&str>) -> Result<Model, ModelsError> {
        self.station
            .client
            .models()
            .create(CreateModelRequest {
                name: name.to_string(),
                description: description.map(str::to_string),
            })
            .map(model_from_wire)
            .map_err(station_failure)
    }

    fn publish_version(
        &self,
        model: &str,
        files: &[VersionFile],
        contents: &dyn MultipartUploadSource,
        metadata: Option<&serde_json::Value>,
        mut observer: &mut dyn TransferObserver,
    ) -> Result<ModelVersion, ModelsError> {
        let models = self.station.client.models();
        let request = UploadModelVersionRequest {
            files: files
                .iter()
                .map(|file| UploadModelFileSpecRequest {
                    rel_path: file.rel_path.clone(),
                    size_bytes: file.size_bytes,
                    checksum: file.checksum.clone(),
                })
                .collect(),
            metadata: metadata.cloned(),
        };
        let planned = models
            .upload_version(model, request)
            .map_err(|error| map_model_error(error, model))?;

        let uploads = planned
            .files
            .into_iter()
            .map(|file| MultipartUploadFile {
                rel_path: file.rel_path,
                parts: file
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
            &self.station.transfer_client,
            &contents,
            &uploads,
            &mut observer,
        )
        .map_err(model_upload_failure)?;

        let version = VersionSpec::Exact(VersionId::new(planned.version.to_string()));
        models
            .complete_version_upload(model, planned.version)
            .map_err(|error| map_version_error(error, model, &version))?;

        models
            .version(model, planned.version)
            .map(model_version_from_wire)
            .map_err(|error| map_version_error(error, model, &version))
    }
}

struct StationVersionFileSource {
    file: VersionFile,
    url: String,
    transfer_client: ReqwestTransferClient,
}

impl VersionFileSource for StationVersionFileSource {
    fn file(&self) -> &VersionFile {
        &self.file
    }

    fn open(&self, _canonical_path: &str) -> Result<VersionFileReader, ModelsError> {
        self.transfer_client
            .get_reader(&self.url, Some(self.file.size_bytes))
            .map_err(|error| ModelsError::Transport(error.to_string()))
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
            Box::new(StationVersionFileSource {
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

fn models_from_wire(response: ModelListResponse) -> Vec<Model> {
    response.items.into_iter().map(model_from_wire).collect()
}

fn model_from_wire(response: ModelResponse) -> Model {
    Model {
        id: response.id,
        name: response.name,
        description: response.description,
        published_by: None,
        created_at: station_timestamp(&response.created_at),
        version_count: response.version_count,
        latest_version: response.latest_version,
    }
}

fn model_versions_from_wire(response: ModelVersionListResponse) -> Vec<ModelVersion> {
    response
        .items
        .into_iter()
        .map(model_version_from_wire)
        .collect()
}

fn model_version_from_wire(response: ModelVersionResponse) -> ModelVersion {
    ModelVersion {
        id: VersionId::new(response.version.to_string()),
        version: Some(response.version),
        state: state_from_wire(response.state),
        failure_reason: response.failure_reason,
        size_bytes: response.size,
        checksum: response.digest,
        aliases: response.aliases,
        published_by: None,
        created_at: station_timestamp(&response.created_at),
        metadata: response.metadata,
        deleted_at: response.deleted_at.as_deref().and_then(station_timestamp),
        manifest: VersionManifest {
            files: response
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
    }
}

fn model_upload_failure(error: UploadError) -> ModelsError {
    match error {
        UploadError::Cancelled { .. } => ModelsError::Cancelled,
        error @ UploadError::Transfer { .. } => ModelsError::Transport(error.to_string()),
        error => ModelsError::other(error),
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

fn station_failure(error: ClientError) -> ModelsError {
    match StationError::from(error) {
        StationError::Transport(reason) => ModelsError::Transport(reason),
        error => ModelsError::other(error),
    }
}

fn map_model_error(error: ClientError, name: &str) -> ModelsError {
    if let Some(refusal) = refusal(&error, name, None) {
        return refusal;
    }
    if error.is_not_found() {
        return ModelsError::ModelNotFound {
            name: name.to_string(),
        };
    }
    station_failure(error)
}

fn map_version_error(error: ClientError, model: &str, version: &VersionSpec) -> ModelsError {
    if let Some(refusal) = refusal(&error, model, Some(version)) {
        return refusal;
    }
    if error.is_not_found() {
        return ModelsError::VersionNotFound {
            model: model.to_string(),
            version: version.clone(),
        };
    }
    station_failure(error)
}

fn map_error(error: ClientError, model: &str, version: Option<&VersionSpec>) -> ModelsError {
    refusal(&error, model, version).unwrap_or_else(|| station_failure(error))
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
