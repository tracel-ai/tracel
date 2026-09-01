use std::sync::Arc;

use serde::Deserialize;
use tracel_artifact::TransferObserver;
use tracel_artifact::upload::{
    MultipartUploadFile, MultipartUploadPart, MultipartUploadSource,
    upload_bundle_multipart_with_client_and_observer,
};
use tracel_artifact::{FileTransferClient, ReqwestTransferClient};
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
    Model, ModelOps, ModelVersion, ModelsError, VersionFile, VersionFileReader, VersionFileSource,
    VersionId, VersionManifest, VersionSpec,
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
        model_versions_from_wire(response)
    }

    fn get_version(&self, model: &str, spec: VersionSpec) -> Result<ModelVersion, ModelsError> {
        let id = match &spec {
            VersionSpec::Exact(id) => id.clone(),
            VersionSpec::Latest => self
                .list_versions(model)?
                .into_iter()
                .max_by_key(|version| version.version)
                .map(|version| version.id)
                .ok_or_else(|| ModelsError::VersionNotFound {
                    model: model.to_string(),
                    version: spec.clone(),
                })?,
        };

        let route = self.route_version(model, &id)?;
        self.scope
            .console
            .client
            .get_model_version(&self.scope.owner, &self.scope.project, model, route)
            .map_err(|error| map_version_error(error, model, &id))
            .and_then(model_version_from_wire)
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
            .map_err(|error| map_version_error(error, model, id))?;
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
        .map_err(ModelsError::other)?;

        self.scope
            .console
            .client
            .complete_model_version_upload(
                &self.scope.owner,
                &self.scope.project,
                model,
                planned.version,
            )
            .map_err(|error| map_model_error(error, model))?;

        self.scope
            .console
            .client
            .get_model_version(
                &self.scope.owner,
                &self.scope.project,
                model,
                planned.version,
            )
            .map_err(|error| map_model_error(error, model))
            .and_then(model_version_from_wire)
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
            .get_reader(&self.url)
            .map_err(|error| ModelsError::other(ConsoleError::Transport(error.to_string())))
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

/// Hands a client failure to the model domain as this console's own.
fn console_failure(error: ClientError) -> ModelsError {
    ModelsError::other(ConsoleError::from(error))
}
