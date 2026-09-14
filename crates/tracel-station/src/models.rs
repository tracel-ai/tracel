use std::sync::Arc;

use tracel_artifact::{ByteStream, HttpTransferClient, TransferClient};
use tracel_client::station::model::request::CreateModelRequest;
use tracel_client::station::model::response::{
    ModelDownloadResponse, ModelListResponse, ModelResponse, ModelVersionListResponse,
    ModelVersionResponse,
};
use tracel_models::{
    Model, ModelOps, ModelVersion, ModelsError, VersionFile, VersionFileSource, VersionId,
    VersionManifest, VersionSpec,
};
use tracel_task::{DynFuture, Task};

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

/// Station calls block until `tracel-client` is asynchronous, so each runs on the executor's
/// blocking lane; bytes go straight to the transport.
impl ModelOps for StationModelOps {
    fn list_models(&self) -> DynFuture<'_, Result<Vec<Model>, ModelsError>> {
        Box::pin(async move {
            let client = self.station.client.clone();
            let response = Task::spawn_blocking(&*self.station.spawn, move || {
                client.models().list().map_err(station_failure)
            })
            .await?;
            Ok(models_from_wire(response))
        })
    }

    fn get_model<'a>(&'a self, name: &'a str) -> DynFuture<'a, Result<Model, ModelsError>> {
        Box::pin(async move {
            let client = self.station.client.clone();
            let name = name.to_string();
            Task::spawn_blocking(&*self.station.spawn, move || {
                client
                    .models()
                    .get(&name)
                    .map(model_from_wire)
                    .map_err(|error| map_model_error(error, &name))
            })
            .await
        })
    }

    fn list_versions<'a>(
        &'a self,
        model: &'a str,
    ) -> DynFuture<'a, Result<Vec<ModelVersion>, ModelsError>> {
        Box::pin(async move {
            let client = self.station.client.clone();
            let model = model.to_string();
            let response = Task::spawn_blocking(&*self.station.spawn, move || {
                client
                    .models()
                    .versions(&model)
                    .map_err(|error| map_model_error(error, &model))
            })
            .await?;
            Ok(model_versions_from_wire(response))
        })
    }

    fn get_version<'a>(
        &'a self,
        model: &'a str,
        spec: VersionSpec,
    ) -> DynFuture<'a, Result<ModelVersion, ModelsError>> {
        Box::pin(async move {
            let id = match &spec {
                VersionSpec::Exact(id) => id.clone(),
                // The Station has no latest-version route, so the listing answers it.
                VersionSpec::Latest => {
                    return self
                        .list_versions(model)
                        .await?
                        .into_iter()
                        .max_by_key(|version| version.version)
                        .ok_or_else(|| ModelsError::VersionNotFound {
                            model: model.to_string(),
                            version: spec,
                        });
                }
            };

            let route = self.route_version(model, &id)?;
            let client = self.station.client.clone();
            let model = model.to_string();
            Task::spawn_blocking(&*self.station.spawn, move || {
                client
                    .models()
                    .version(&model, route)
                    .map(model_version_from_wire)
                    .map_err(|error| map_version_error(error, &model, &id))
            })
            .await
        })
    }

    fn fetch_version_files<'a>(
        &'a self,
        model: &'a str,
        id: &'a VersionId,
    ) -> DynFuture<'a, Result<Vec<Box<dyn VersionFileSource>>, ModelsError>> {
        Box::pin(async move {
            let route = self.route_version(model, id)?;
            let client = self.station.client.clone();
            let model = model.to_string();
            let id = id.clone();
            let response = Task::spawn_blocking(&*self.station.spawn, move || {
                client
                    .models()
                    .download(&model, route)
                    .map_err(|error| map_version_error(error, &model, &id))
            })
            .await?;
            Ok(file_sources_from_wire(&self.station.transfer, response))
        })
    }

    fn create_model<'a>(
        &'a self,
        name: &'a str,
        description: Option<&'a str>,
    ) -> DynFuture<'a, Result<Model, ModelsError>> {
        Box::pin(async move {
            let client = self.station.client.clone();
            let request = CreateModelRequest {
                name: name.to_string(),
                description: description.map(str::to_string),
            };
            Task::spawn_blocking(&*self.station.spawn, move || {
                client
                    .models()
                    .create(request)
                    .map(model_from_wire)
                    .map_err(station_failure)
            })
            .await
        })
    }

    fn publish_version<'a>(
        &'a self,
        _model: &'a str,
        _files: &'a [VersionFile],
        _contents: &'a dyn tracel_artifact::upload::MultipartUploadSource,
        _metadata: Option<&'a serde_json::Value>,
        _observer: &'a mut dyn tracel_artifact::TransferObserver,
    ) -> DynFuture<'a, Result<ModelVersion, ModelsError>> {
        Box::pin(async move {
            Err(ModelsError::other(
                "publishing a model version is not implemented for the station yet",
            ))
        })
    }
}

struct StationVersionFileSource {
    file: VersionFile,
    url: String,
    transfer: HttpTransferClient,
}

impl VersionFileSource for StationVersionFileSource {
    fn file(&self) -> &VersionFile {
        &self.file
    }

    fn open<'a>(
        &'a self,
        _canonical_path: &'a str,
    ) -> DynFuture<'a, Result<ByteStream, ModelsError>> {
        Box::pin(async move {
            self.transfer
                .get(&self.url, Some(self.file.size_bytes))
                .await
                .map_err(|error| ModelsError::Transport(error.to_string()))
        })
    }
}

fn file_sources_from_wire(
    transfer: &HttpTransferClient,
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
                transfer: transfer.clone(),
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
        latest_version: None,
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
        size_bytes: response.size,
        checksum: response.checksum,
        published_by: None,
        created_at: station_timestamp(&response.created_at),
        // The Station's version response carries no metadata.
        metadata: serde_json::Value::Null,
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

fn station_failure(error: tracel_client::ClientError) -> ModelsError {
    match StationError::from(error) {
        StationError::Transport(reason) => ModelsError::Transport(reason),
        error => ModelsError::other(error),
    }
}

fn map_model_error(error: tracel_client::ClientError, name: &str) -> ModelsError {
    if error.is_not_found() {
        return ModelsError::ModelNotFound {
            name: name.to_string(),
        };
    }
    station_failure(error)
}

fn map_version_error(
    error: tracel_client::ClientError,
    model: &str,
    id: &VersionId,
) -> ModelsError {
    if error.is_not_found() {
        return ModelsError::VersionNotFound {
            model: model.to_string(),
            version: VersionSpec::Exact(id.clone()),
        };
    }
    station_failure(error)
}
