use std::sync::Mutex;
use std::time::Duration;

use futures::future::{Either, select};
use futures_timer::Delay;
use tracel_experiment::error::{ExperimentError, ExperimentErrorKind};
use tracel_experiment::session::{BundleFn, Event, ExperimentCompletion, ExperimentSession};
use tracel_experiment::{
    ActivityEvent, ActivityId, ActivityStatus, ArtifactKind, LogLevel, LogRecord, MetricSpec,
    MetricValue,
};

use tracel_artifact::bundle::FsBundle;
use tracel_client::websocket::{
    ActivityEventRequest, ActivityMeterRequest, ActivityRequest, ActivityStatusRequest,
    ExperimentCompletion as RemoteExperimentCompletion, ExperimentMessage, InputUsed, LogEntry,
    LogEntryLevel, MetricLog,
};

use crate::actor::SocketHandle;

/// An artifact that could not be handed to the backend.
#[derive(Debug, thiserror::Error)]
#[error("Failed to upload artifact: {message}")]
pub struct ArtifactUploadError {
    /// What went wrong.
    pub message: String,
    /// The backend's own error, when it reported one.
    #[source]
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

/// Sends a run's artifacts wherever the backend keeps them.
pub trait ArtifactUploader {
    /// Uploads one bundle under `name`.
    fn upload(
        &self,
        name: &str,
        kind: ArtifactKind,
        bundle: &FsBundle,
    ) -> Result<(), ArtifactUploadError>;
}

/// An [`ArtifactUploader`] a session can own.
pub type BoxedArtifactUploader = Box<dyn ArtifactUploader + Send + Sync>;

/// An [`ExperimentSession`] that speaks the Tracel remote experiment protocol over a websocket.
pub struct RemoteExperimentSession {
    artifact_uploader: BoxedArtifactUploader,
    socket: Mutex<Option<SocketHandle>>,
}

impl RemoteExperimentSession {
    /// Opens a session over an already-running socket actor, handing artifacts to
    /// `artifact_uploader`.
    pub fn new(artifact_uploader: BoxedArtifactUploader, socket: SocketHandle) -> Self {
        Self {
            artifact_uploader,
            socket: Mutex::new(Some(socket)),
        }
    }

    fn send(&self, message: ExperimentMessage) -> Result<(), ExperimentError> {
        let guard = self.socket.lock().unwrap();
        let socket = guard.as_ref().ok_or_else(|| {
            ExperimentError::new(
                ExperimentErrorKind::AlreadyFinished,
                "Experiment run has already finished",
            )
        })?;

        socket.send(message).map_err(|_| {
            ExperimentError::new(
                ExperimentErrorKind::Internal,
                "Failed to send message to experiment session",
            )
        })
    }
}

/// Only a dead connection waits this out; a live one answers as soon as its queue is written.
const FLUSH_TIMEOUT: Duration = Duration::from_secs(2);

impl ExperimentSession for RemoteExperimentSession {
    fn record_event(&self, event: Event) -> Result<(), ExperimentError> {
        self.send(to_remote_message(event))
    }

    fn flush(&self) -> Result<(), ExperimentError> {
        let flushed = {
            let guard = self.socket.lock().unwrap();
            let Some(socket) = guard.as_ref() else {
                // A finished session already drained on the way out.
                return Ok(());
            };
            socket.flush()
        };

        let outcome = futures::executor::block_on(select(flushed, Delay::new(FLUSH_TIMEOUT)));
        match outcome {
            Either::Left((Ok(()), _)) => Ok(()),
            Either::Left((Err(_), _)) => Err(ExperimentError::new(
                ExperimentErrorKind::Internal,
                "The experiment socket is no longer accepting events",
            )),
            Either::Right(_) => Err(ExperimentError::new(
                ExperimentErrorKind::Internal,
                "The experiment socket did not confirm delivery in time",
            )),
        }
    }

    fn save_artifact(
        &self,
        name: &str,
        kind: ArtifactKind,
        artifact: Box<BundleFn>,
    ) -> Result<(), ExperimentError> {
        let mut bundle = FsBundle::temp().map_err(|err| {
            ExperimentError::with_source(
                ExperimentErrorKind::Artifact,
                "Failed to create temporary bundle for artifact upload",
                err,
            )
        })?;

        artifact(&mut bundle)?;

        self.artifact_uploader
            .upload(name, kind, &bundle)
            .map_err(|err| {
                ExperimentError::with_source(
                    ExperimentErrorKind::Artifact,
                    "Failed to upload experiment artifact",
                    err,
                )
            })
    }

    fn finish(&self, completion: ExperimentCompletion) -> Result<(), ExperimentError> {
        let socket = self.socket.lock().unwrap().take().ok_or_else(|| {
            ExperimentError::new(
                ExperimentErrorKind::AlreadyFinished,
                "Experiment run has already finished",
            )
        })?;

        socket
            .send(ExperimentMessage::ExperimentComplete(to_remote_completion(
                completion,
            )))
            .map_err(|_| {
                ExperimentError::new(
                    ExperimentErrorKind::Internal,
                    "Failed to send experiment completion to remote session",
                )
            })?;

        if let Err(error) = socket.close().block() {
            tracing::warn!("WebSocket failure during experiment finish: {error}");
        }

        Ok(())
    }
}

fn to_remote_message(event: Event) -> ExperimentMessage {
    match event {
        Event::Args(value) => ExperimentMessage::Arguments(value),
        Event::Config { name, value } => ExperimentMessage::Config { name, value },
        Event::Log { record, activity } => {
            ExperimentMessage::LogEntries(vec![to_log_entry(record, activity)])
        }
        Event::Metrics {
            epoch,
            split,
            iteration,
            items,
            activity,
        } => ExperimentMessage::MetricsLog {
            epoch,
            split,
            iteration,
            items: to_remote_metric_logs(items),
            activity: to_remote_activity_id(activity),
        },
        Event::MetricDefinition(MetricSpec {
            name,
            description,
            unit,
            higher_is_better,
        }) => ExperimentMessage::MetricDefinitionLog {
            name,
            description,
            unit,
            higher_is_better,
        },
        Event::EpochSummary {
            epoch,
            split,
            items,
            activity,
        } => ExperimentMessage::EpochSummaryLog {
            epoch,
            split,
            best_metric_values: to_remote_metric_logs(items),
            activity: to_remote_activity_id(activity),
        },
        Event::Summary { items, activity } => ExperimentMessage::SummaryLog {
            items: to_remote_metric_logs(items),
            activity: to_remote_activity_id(activity),
        },
        Event::ArtifactUsed {
            experiment_id: _,
            reference,
        } => ExperimentMessage::InputUsed(InputUsed::Artifact {
            artifact_id: reference.id,
        }),
        Event::Activity(activity_event) => {
            ExperimentMessage::Activity(to_remote_activity_event(activity_event))
        }
    }
}

fn to_log_entry(record: LogRecord, activity: Option<ActivityId>) -> LogEntry {
    LogEntry {
        timestamp: chrono::Utc::now().to_rfc3339(),
        level: to_wire_log_level(record.level),
        message: record.message,
        metadata: record.attributes,
        activity: to_remote_activity_id(activity),
    }
}

fn to_remote_activity_id(activity: Option<ActivityId>) -> Option<u64> {
    activity.map(ActivityId::as_u64)
}

fn to_wire_log_level(level: LogLevel) -> LogEntryLevel {
    match level {
        LogLevel::Trace => LogEntryLevel::Trace,
        LogLevel::Debug => LogEntryLevel::Debug,
        LogLevel::Info => LogEntryLevel::Info,
        LogLevel::Warn => LogEntryLevel::Warn,
        LogLevel::Error => LogEntryLevel::Error,
    }
}

fn to_remote_metric_logs(items: Vec<MetricValue>) -> Vec<MetricLog> {
    items
        .into_iter()
        .map(|item| MetricLog {
            name: item.name,
            value: item.value,
        })
        .collect()
}

fn to_remote_activity_event(event: ActivityEvent) -> ActivityEventRequest {
    match event {
        ActivityEvent::Started { activity: spec } => ActivityEventRequest::Started {
            activity: ActivityRequest {
                id: spec.id.as_u64(),
                parent: spec.parent.map(|parent| parent.as_u64()),
                name: spec.name,
                cancellable: spec.cancellable,
                meter: spec.meter.map(|meter| ActivityMeterRequest {
                    unit: meter.unit,
                    total: meter.total,
                }),
                attributes: spec.attributes,
            },
        },
        ActivityEvent::Updated { id, current } => ActivityEventRequest::Updated {
            id: id.as_u64(),
            current,
        },
        ActivityEvent::Message { id, message } => ActivityEventRequest::Message {
            id: id.as_u64(),
            message,
        },
        ActivityEvent::Finished {
            id,
            status,
            message,
        } => ActivityEventRequest::Finished {
            id: id.as_u64(),
            status: match status {
                ActivityStatus::Success => ActivityStatusRequest::Success,
                ActivityStatus::Abandoned => ActivityStatusRequest::Abandoned,
                ActivityStatus::Failed => ActivityStatusRequest::Failed,
            },
            message,
        },
    }
}

fn to_remote_completion(completion: ExperimentCompletion) -> RemoteExperimentCompletion {
    match completion {
        ExperimentCompletion::Success => RemoteExperimentCompletion::Success,
        ExperimentCompletion::Failed(reason) => RemoteExperimentCompletion::Fail { reason },
        ExperimentCompletion::Cancelled => RemoteExperimentCompletion::Success,
    }
}
