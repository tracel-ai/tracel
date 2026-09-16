use std::future::Future;
use std::sync::Mutex;
use std::time::Duration;

use async_channel::{Receiver, Sender};
use futures::channel::oneshot;
use futures::future::{Either, select};
use futures_timer::Delay;
use tracel_experiment::error::{ExperimentError, ExperimentErrorKind};
use tracel_experiment::session::{Event, ExperimentCompletion, ExperimentSession};
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
use tracel_task::{Job, MaybeSend};

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
        name: String,
        kind: ArtifactKind,
        bundle: FsBundle,
    ) -> Job<(), ArtifactUploadError>;
}

/// An [`ArtifactUploader`] a session can own.
pub type BoxedArtifactUploader = Box<dyn ArtifactUploader + Send + Sync>;

/// An [`ExperimentSession`] that speaks the Tracel remote experiment protocol over a websocket.
///
/// Events go straight to the socket actor. Artifacts and the completion go through a second
/// actor, which uploads bundles one at a time in the order they were saved and sends the
/// completion only once every bundle before it has landed. Both actors are run by the backend.
pub struct RemoteExperimentSession {
    live: Mutex<Option<Live>>,
}

/// What a session holds until it finishes.
struct Live {
    socket: SocketHandle,
    shipper: Sender<Request>,
}

impl RemoteExperimentSession {
    /// Creates the session and the loop that ships its artifacts through `artifact_uploader`
    /// and completes it over `socket`, which must already be running. The caller runs the loop
    /// wherever its loops run.
    pub fn start(
        artifact_uploader: BoxedArtifactUploader,
        socket: SocketHandle,
    ) -> (Self, impl Future<Output = ()> + MaybeSend + 'static) {
        let (shipper, requests) = async_channel::unbounded();
        let session = Self {
            live: Mutex::new(Some(Live {
                socket: socket.clone(),
                shipper,
            })),
        };
        (session, ship(artifact_uploader, socket, requests))
    }

    fn send(&self, message: ExperimentMessage) -> Result<(), ExperimentError> {
        let guard = self.live.lock().unwrap();
        let live = guard.as_ref().ok_or_else(already_finished)?;

        live.socket.send(message).map_err(|_| {
            ExperimentError::new(
                ExperimentErrorKind::Internal,
                "Failed to send message to experiment session",
            )
        })
    }
}

/// Queues a request carrying a reply slot; a shipper that stopped first answers as stopped.
fn ask(
    shipper: &Sender<Request>,
    request: impl FnOnce(Reply) -> Request,
) -> Job<(), ExperimentError> {
    let (reply, answer) = oneshot::channel();
    match shipper.try_send(request(reply)) {
        Ok(()) => Job::new(async move { answer.await.unwrap_or_else(|_| Err(shipper_stopped())) }),
        Err(_) => Job::failed(shipper_stopped()),
    }
}

fn already_finished() -> ExperimentError {
    ExperimentError::new(
        ExperimentErrorKind::AlreadyFinished,
        "Experiment run has already finished",
    )
}

fn shipper_stopped() -> ExperimentError {
    ExperimentError::new(
        ExperimentErrorKind::Internal,
        "The experiment session stopped before it could answer",
    )
}

/// Only a dead connection waits this out; a live one answers as soon as its queue is written.
const FLUSH_TIMEOUT: Duration = Duration::from_secs(2);

impl ExperimentSession for RemoteExperimentSession {
    fn record_event(&self, event: Event) -> Result<(), ExperimentError> {
        self.send(to_remote_message(event))
    }

    fn flush(&self) -> Job<(), ExperimentError> {
        let guard = self.live.lock().unwrap();
        let Some(live) = guard.as_ref() else {
            // A finished session already drained on the way out.
            return Job::ready(());
        };
        ask(&live.shipper, Request::Flush)
    }

    fn save_artifact(
        &self,
        name: String,
        kind: ArtifactKind,
        bundle: FsBundle,
    ) -> Result<(), ExperimentError> {
        let guard = self.live.lock().unwrap();
        let live = guard.as_ref().ok_or_else(already_finished)?;

        live.shipper
            .try_send(Request::Save { name, kind, bundle })
            .map_err(|_| shipper_stopped())
    }

    fn finish(&self, completion: ExperimentCompletion) -> Job<(), ExperimentError> {
        let Some(live) = self.live.lock().unwrap().take() else {
            return Job::failed(already_finished());
        };
        ask(&live.shipper, |reply| Request::Finish(completion, reply))
    }
}

type Reply = oneshot::Sender<Result<(), ExperimentError>>;

enum Request {
    Save {
        name: String,
        kind: ArtifactKind,
        bundle: FsBundle,
    },
    Flush(Reply),
    Finish(ExperimentCompletion, Reply),
}

/// Ships artifacts in the order they were saved, then the completion.
///
/// One request at a time: a flush or finish is answered only after every upload queued before
/// it has been driven to its end, so nothing here needs to count what is in flight.
async fn ship(
    artifact_uploader: BoxedArtifactUploader,
    socket: SocketHandle,
    requests: Receiver<Request>,
) {
    let mut failed: Option<String> = None;

    while let Ok(request) = requests.recv().await {
        match request {
            Request::Save { name, kind, bundle } => {
                if let Err(error) = artifact_uploader.upload(name.clone(), kind, bundle).await {
                    tracing::error!(artifact = %name, error = %error, "The artifact did not reach the backend");
                    failed.get_or_insert(format!("{error}"));
                }
            }
            Request::Flush(reply) => {
                let flushed = flush_socket(&socket).await;
                let _ = reply.send(flushed.and_then(|()| shipped(&failed)));
            }
            Request::Finish(completion, reply) => {
                let sent = socket
                    .send(ExperimentMessage::ExperimentComplete(to_remote_completion(
                        completion,
                    )))
                    .map_err(|_| {
                        ExperimentError::new(
                            ExperimentErrorKind::Internal,
                            "Failed to send experiment completion to remote session",
                        )
                    });
                // Queued behind the completion, so the reply means the socket closed after it
                // was written.
                if let Err(error) = socket.close().await {
                    tracing::warn!("WebSocket failure during experiment finish: {error}");
                }
                let _ = reply.send(sent.and_then(|()| shipped(&failed)));
                return;
            }
        }
    }
}

async fn flush_socket(socket: &SocketHandle) -> Result<(), ExperimentError> {
    match select(socket.flush(), Delay::new(FLUSH_TIMEOUT)).await {
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

/// The first upload failure, reported by every flush and by the finish after it.
fn shipped(failed: &Option<String>) -> Result<(), ExperimentError> {
    match failed {
        None => Ok(()),
        Some(message) => Err(ExperimentError::new(
            ExperimentErrorKind::Artifact,
            message.clone(),
        )),
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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tracel_experiment::ExperimentRunControl;

    use super::*;
    use crate::test_support::{Peer, definition, fake_socket, holds_within, started};

    /// Records uploads by name; the first one waits on `gate` when there is one, and the one
    /// named `failing` is refused.
    struct FakeUploader {
        done: Arc<Mutex<Vec<String>>>,
        gate: Mutex<Option<oneshot::Receiver<()>>>,
        failing: Option<String>,
    }

    impl ArtifactUploader for FakeUploader {
        fn upload(
            &self,
            name: String,
            _kind: ArtifactKind,
            _bundle: FsBundle,
        ) -> Job<(), ArtifactUploadError> {
            if self.failing.as_deref() == Some(name.as_str()) {
                return Job::failed(ArtifactUploadError {
                    message: format!("no room for '{name}'"),
                    source: None,
                });
            }
            let gate = self.gate.lock().unwrap().take();
            let done = Arc::clone(&self.done);
            Job::new(async move {
                if let Some(gate) = gate {
                    let _ = gate.await;
                }
                done.lock().unwrap().push(name);
                Ok(())
            })
        }
    }

    struct Setup {
        session: RemoteExperimentSession,
        peer: Peer,
        done: Arc<Mutex<Vec<String>>>,
    }

    /// Starts a session with both loops driven on threads of their own.
    fn setup(gate: Option<oneshot::Receiver<()>>, failing: Option<&str>) -> Setup {
        let done = Arc::new(Mutex::new(Vec::new()));
        let uploader = FakeUploader {
            done: Arc::clone(&done),
            gate: Mutex::new(gate),
            failing: failing.map(str::to_string),
        };
        let (socket, peer) = fake_socket(None);
        let socket = started(socket, ExperimentRunControl::default());
        let (session, ship) = RemoteExperimentSession::start(Box::new(uploader), socket);
        std::thread::spawn(move || futures::executor::block_on(ship));
        Setup {
            session,
            peer,
            done,
        }
    }

    fn save(session: &RemoteExperimentSession, name: &str) -> Result<(), ExperimentError> {
        session.save_artifact(
            name.to_string(),
            ArtifactKind::Other,
            FsBundle::temp().unwrap(),
        )
    }

    #[test]
    fn saving_never_waits_and_artifacts_ship_in_the_order_they_were_saved() {
        let (open, gate) = oneshot::channel();
        let Setup {
            session,
            peer: _peer,
            done,
        } = setup(Some(gate), None);

        save(&session, "a").unwrap();
        save(&session, "b").unwrap();
        save(&session, "c").unwrap();
        assert!(done.lock().unwrap().is_empty());

        open.send(()).unwrap();
        session.flush().block().unwrap();

        assert_eq!(done.lock().unwrap().as_slice(), ["a", "b", "c"]);
    }

    #[test]
    fn flush_resolves_only_once_the_artifacts_queued_before_it_have_shipped() {
        let (open, gate) = oneshot::channel();
        let Setup {
            session,
            peer: _peer,
            done,
        } = setup(Some(gate), None);

        save(&session, "a").unwrap();
        let mut flushing = session.flush();
        assert!(flushing.try_poll().is_none());

        open.send(()).unwrap();

        flushing.block().unwrap();
        assert_eq!(done.lock().unwrap().as_slice(), ["a"]);
    }

    #[test]
    fn the_completion_is_sent_only_after_every_artifact_has_shipped() {
        let (open, gate) = oneshot::channel();
        let Setup {
            session,
            peer,
            done,
        } = setup(Some(gate), None);

        session
            .record_event(Event::MetricDefinition(spec("loss")))
            .unwrap();
        save(&session, "model").unwrap();
        let mut finishing = session.finish(ExperimentCompletion::Success);

        assert!(holds_within(5, || peer.sent() == ["loss"]));
        assert!(finishing.try_poll().is_none());
        assert!(!peer.closing());

        open.send(()).unwrap();

        finishing.block().unwrap();
        assert_eq!(done.lock().unwrap().as_slice(), ["model"]);
        assert_eq!(peer.sent(), ["loss", "complete:Success"]);
        assert!(peer.closed());
    }

    #[test]
    fn an_artifact_that_fails_to_ship_is_reported_by_flush_and_finish_and_the_run_still_completes()
    {
        let Setup {
            session,
            peer,
            done,
        } = setup(None, Some("a"));

        save(&session, "a").unwrap();
        save(&session, "b").unwrap();

        let flushed = session.flush().block().unwrap_err();
        assert_eq!(flushed.kind, ExperimentErrorKind::Artifact);
        assert!(flushed.message.contains("no room for 'a'"), "{flushed}");
        assert_eq!(done.lock().unwrap().as_slice(), ["b"]);

        let finished = session
            .finish(ExperimentCompletion::Success)
            .block()
            .unwrap_err();
        assert_eq!(finished.kind, ExperimentErrorKind::Artifact);
        assert_eq!(peer.sent(), ["complete:Success"]);
        assert!(peer.closed());
    }

    #[test]
    fn nothing_is_accepted_after_finish() {
        let Setup {
            session,
            peer: _peer,
            ..
        } = setup(None, None);

        session
            .finish(ExperimentCompletion::Success)
            .block()
            .unwrap();

        assert_eq!(
            save(&session, "late").unwrap_err().kind,
            ExperimentErrorKind::AlreadyFinished
        );
        assert_eq!(
            session
                .record_event(Event::MetricDefinition(spec("late")))
                .unwrap_err()
                .kind,
            ExperimentErrorKind::AlreadyFinished
        );
        assert_eq!(
            session
                .finish(ExperimentCompletion::Success)
                .block()
                .unwrap_err()
                .kind,
            ExperimentErrorKind::AlreadyFinished
        );
        session.flush().block().unwrap();
    }

    #[test]
    fn a_finish_nobody_drives_still_completes_the_run() {
        let Setup {
            session,
            peer,
            done,
        } = setup(None, None);

        save(&session, "model").unwrap();
        drop(session.finish(ExperimentCompletion::Cancelled));

        assert!(holds_within(5, || peer.closed()));
        assert_eq!(done.lock().unwrap().as_slice(), ["model"]);
        assert_eq!(peer.sent(), ["complete:Success"]);
    }

    fn spec(name: &str) -> MetricSpec {
        let ExperimentMessage::MetricDefinitionLog {
            name,
            description,
            unit,
            higher_is_better,
        } = definition(name)
        else {
            unreachable!()
        };
        MetricSpec {
            name,
            description,
            unit,
            higher_is_better,
        }
    }
}
