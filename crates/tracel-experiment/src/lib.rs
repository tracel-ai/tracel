//! Experiment tracking primitives.
//!
//! The core APIs are:
//! - [`ExperimentRun`], which owns the lifecycle of an active experiment.
//! - [`ExperimentRunHandle`], which is a lightweight cloneable view for logging and artifact access
//!   from background tasks or other threads.
//!
//! Borrowed runs, handles, activities, and guards convert into scope-carrying handles through
//! `Into<ExperimentRunHandle>`.
//!
//! Telemetry is fire-and-forget: logs, metrics, summaries, and activity progress return `()` and
//! are discarded when the run they target has finished or been dropped. A `Result` therefore means
//! the caller's own data or the artifact store had a problem worth acting on — a value that failed
//! to serialize, or an artifact that could not be encoded, stored, or loaded — never that the
//! telemetry sink was unavailable. Use [`ExperimentRunHandle::is_active`] to branch on liveness.
//!
//! Optional capabilities are exposed through extension traits:
//! - [`ExperimentGlobalExt`] for ambient thread-local experiment context.
//! - [`integration::training::ExperimentTrainingExt`] for Burn `train` adapters.
//! - [`integration::training::SupervisedTrainingExperimentExt`] for one-line Burn builder wiring.
//! - [`integration::tracing::ExperimentTracingExt`] for tracing span helpers.
//!
//! [`ExperimentModule`] accepts a backend experiment-creation callback and builds
//! [`ExperimentJob`]s, the user-facing entry points for running a job and logging its result.

use std::fmt;
use std::str::FromStr;
use std::sync::{Arc, Mutex, Weak};

use serde::Serialize;
use tracel_artifact::bundle::{BundleDecode, BundleEncode, FsBundle};

mod activity;
mod cancellation;
mod context;
mod control;
mod log;
mod panic_watch;
mod provider;
pub mod reader;
pub mod session;
#[cfg(test)]
mod test_support;

pub mod error;
pub mod integration;

pub use activity::{
    Activity, ActivityBuilder, ActivityEvent, ActivityGuard, ActivityId, ActivityMeter,
    ActivitySpec, ActivityStatus,
};
pub use cancellation::{CancelToken, Cancellable};
pub use context::{
    CurrentExperimentGuard, ExperimentGlobalExt, ExperimentInstrument, WithCurrentExperiment,
};
pub use control::ExperimentRunControl;
pub use log::{LogLevel, LogRecord};
pub use panic_watch::PanicWatch;
pub use provider::{ExperimentFn, ExperimentJob, ExperimentModule};

use crate::activity::AtomicActivityIdAllocator;
use crate::error::{ExperimentError, ExperimentErrorKind};
use crate::integration::tracing::registry::{TracingRegistration, TracingRegistry};
use crate::reader::ExperimentArtifactReader;
use crate::session::{Event, ExperimentCompletion, ExperimentSession};

/// Opaque identifier for an experiment run.
///
/// The identifier format is backend-specific.
///
/// It is stable for the backend that created it, but it should not be interpreted across different backends.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExperimentId(String);

impl ExperimentId {
    /// Create an experiment identifier from a backend-specific string value.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Borrow the backend-specific identifier value.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Try to parse the identifier value into another type.
    pub fn parse<T: FromStr>(&self) -> Option<T> {
        self.0.parse().ok()
    }
}

impl fmt::Display for ExperimentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for ExperimentId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for ExperimentId {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

impl From<&String> for ExperimentId {
    fn from(value: &String) -> Self {
        Self(value.clone())
    }
}

impl From<i32> for ExperimentId {
    fn from(value: i32) -> Self {
        Self(value.to_string())
    }
}

impl From<u32> for ExperimentId {
    fn from(value: u32) -> Self {
        Self(value.to_string())
    }
}

/// Artifact category associated with an experiment run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactKind {
    /// Model weights, parameters, or checkpoints.
    Model,
    /// Log files or related textual outputs.
    Log,
    /// Any artifact that does not fit a more specific category.
    Other,
}

/// Metric definition metadata logged during a run.
#[derive(Debug, Clone)]
pub struct MetricSpec {
    /// Display name for the metric.
    pub name: String,
    /// Optional human-readable description.
    pub description: Option<String>,
    /// Optional unit associated with the metric value.
    pub unit: Option<String>,
    /// Whether higher values are considered better.
    pub higher_is_better: bool,
}

/// Numeric metric value logged during a run.
#[derive(Debug, Clone)]
pub struct MetricValue {
    /// Metric name.
    pub name: String,
    /// Metric value.
    pub value: f64,
}

#[derive(Debug, Clone)]
struct ExperimentMetadata {
    pub id: ExperimentId,
}

/// An active experiment run.
///
/// `ExperimentRun` owns finalization. As long as the run remains active, it can log structured
/// events, persist artifacts, and expose a cancellation token to child work.
///
/// Use [`ExperimentRun::handle`] when you need to share logging and artifact access without
/// transferring lifecycle ownership. If the run is dropped without an explicit completion, it is
/// finalized automatically.
///
/// Use [`ExperimentGlobalExt`] when you want to make the run available as the ambient
/// thread-local experiment for tracing or other integrations.
pub struct ExperimentRun {
    inner: Arc<RunInner>,
    handle: ExperimentRunHandle,
    _tracing_registration: TracingRegistration,
}

/// Cloneable handle for interacting with an active experiment run.
///
/// A handle keeps the run identifier plus logging and artifact access, but it does not own the run
/// lifecycle. This makes it the right type to move into async tasks, worker threads, or adapter
/// objects.
///
/// If the originating [`ExperimentRun`] is finished or dropped, existing handles become inactive:
/// telemetry raised through them is discarded, and artifact operations report an error.
#[derive(Clone)]
pub struct ExperimentRunHandle {
    metadata: ExperimentMetadata,
    inner: Weak<RunInner>,
    control: ExperimentRunControl,
    activity: Option<ActivityId>,
    context_cancel_token: CancelToken,
    /// Attributes inherited by every log emitted through this handle. Cloned handles share the
    /// scope until [`ExperimentRunHandle::with_attr`]/[`ExperimentRunHandle::with_attrs`] extends it.
    scope: Arc<serde_json::Map<String, serde_json::Value>>,
}

/// Converting this borrow clones the run's scope-carrying handle, so telemetry emitted through the
/// resulting handle is attributed to that scope.
///
/// Custom context types integrate by implementing `From<&TheirType>` for [`ExperimentRunHandle`].
impl From<&ExperimentRun> for ExperimentRunHandle {
    fn from(value: &ExperimentRun) -> Self {
        value.handle.clone()
    }
}

/// Converting this borrow clones the scope-carrying handle, so telemetry emitted through the
/// resulting handle is attributed to that scope.
///
/// Custom context types integrate by implementing `From<&TheirType>` for [`ExperimentRunHandle`].
impl From<&ExperimentRunHandle> for ExperimentRunHandle {
    fn from(value: &ExperimentRunHandle) -> Self {
        value.clone()
    }
}

struct RunInner {
    control: ExperimentRunControl,
    state: Mutex<RunState>,
    session: Box<dyn ExperimentSession>,
    reader: Box<dyn ExperimentArtifactReader>,
    activity_id_allocator: Arc<AtomicActivityIdAllocator>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunState {
    Active,
    Finished,
}

impl ExperimentRun {
    /// Create a run from backend-specific session and artifact reader implementations.
    ///
    /// This is the low-level constructor used by the built-in local and remote backends. Most
    /// callers should prefer [`Self::local`] or [`Self::remote`].
    pub fn new<S, R>(
        id: impl Into<ExperimentId>,
        session: S,
        reader: R,
        cancel_token: CancelToken,
    ) -> Self
    where
        S: ExperimentSession + 'static,
        R: ExperimentArtifactReader + 'static,
    {
        Self::new_with_control(id, session, reader, ExperimentRunControl::new(cancel_token))
    }

    /// Create a run from backend-specific implementations and an existing control plane.
    ///
    /// Remote backends use this constructor when their socket/control task must receive server
    /// control messages before the run object is returned to user code.
    pub fn new_with_control<S, R>(
        id: impl Into<ExperimentId>,
        session: S,
        reader: R,
        control: ExperimentRunControl,
    ) -> Self
    where
        S: ExperimentSession + 'static,
        R: ExperimentArtifactReader + 'static,
    {
        panic_watch::install();

        let metadata = ExperimentMetadata { id: id.into() };
        let inner = Arc::new(RunInner {
            control: control.clone(),
            state: Mutex::new(RunState::Active),
            session: Box::new(session),
            reader: Box::new(reader),
            activity_id_allocator: Arc::new(AtomicActivityIdAllocator::new()),
        });

        let handle = ExperimentRunHandle {
            metadata,
            inner: Arc::downgrade(&inner),
            context_cancel_token: control.cancel_token(),
            control,
            activity: None,
            scope: Arc::new(serde_json::Map::new()),
        };
        let tracing_registration = TracingRegistry::global().register_handle(handle.clone());

        Self {
            inner,
            handle,
            _tracing_registration: tracing_registration,
        }
    }

    /// Stream every panic on any thread into this run's log until the guard
    /// drops. The failure reason needs no guard: an unwinding run reads the
    /// panicking thread's record on its own.
    pub fn capture_panics(&self) -> PanicWatch {
        panic_watch::watch(self.handle())
    }

    /// Return a cancellation token that can be linked to child work.
    ///
    /// Cancelling the token does not finish the run; it only broadcasts cancellation to linked
    /// tasks and adapters.
    pub fn cancel_token(&self) -> CancelToken {
        self.inner.control.cancel_token()
    }

    /// Signal that the experiment run has been cancelled.
    ///
    /// The run remains usable until it is explicitly finished, failed, or dropped. If it is later
    /// dropped without an explicit completion, it will be marked as cancelled.
    pub fn cancel(&self) -> Result<(), ExperimentError> {
        self.inner.ensure_active()?;
        self.inner.control.cancel_run();
        Ok(())
    }

    /// Mark the run as successful and finalize the backend session.
    ///
    /// If the run is dropped without calling [`Self::finish`] or [`Self::fail`], it is finalized
    /// as successful by default. Any cloned [`ExperimentRunHandle`] becomes inactive afterwards.
    pub fn finish(self) -> Result<(), ExperimentError> {
        self.inner.finish_once(ExperimentCompletion::Success)
    }

    /// Mark the run as failed and finalize the backend session.
    ///
    /// Any cloned [`ExperimentRunHandle`] becomes inactive afterwards.
    pub fn fail(self, reason: impl Into<String>) -> Result<(), ExperimentError> {
        self.inner
            .finish_once(ExperimentCompletion::Failed(reason.into()))
    }

    /// Borrow the identifier for the underlying run.
    pub fn id(&self) -> &ExperimentId {
        self.handle.id()
    }

    /// Log the serialized input arguments for the run.
    ///
    /// Fails only if `args` cannot be serialized.
    pub fn log_args<A: Serialize>(&self, args: &A) -> Result<(), ExperimentError> {
        let value = serde_json::to_value(args).map_err(|error| {
            ExperimentError::with_source(
                ExperimentErrorKind::Artifact,
                "Failed to serialize experiment arguments",
                error,
            )
        })?;

        self.handle.emit(Event::Args(value));
        Ok(())
    }

    /// Log a `trace`-level message.
    pub fn log_trace(&self, message: impl Into<String>) {
        self.handle.log(LogRecord::trace(message));
    }

    /// Log a `debug`-level message.
    pub fn log_debug(&self, message: impl Into<String>) {
        self.handle.log(LogRecord::debug(message));
    }

    /// Log an `info`-level message.
    pub fn log_info(&self, message: impl Into<String>) {
        self.handle.log(LogRecord::info(message));
    }

    /// Log a `warn`-level message.
    pub fn log_warn(&self, message: impl Into<String>) {
        self.handle.log(LogRecord::warn(message));
    }

    /// Log an `error`-level message.
    pub fn log_error(&self, message: impl Into<String>) {
        self.handle.log(LogRecord::error(message));
    }

    /// Record a structured log entry.
    pub fn log(&self, record: LogRecord) {
        self.handle.log(record);
    }

    /// Log a named configuration object.
    pub fn log_config<C: Serialize>(
        &self,
        name: impl Into<String>,
        config: &C,
    ) -> Result<(), ExperimentError> {
        self.handle.log_config(name, config)
    }

    /// Log metric values for an epoch, split, and iteration.
    pub fn log_metric(
        &self,
        epoch: usize,
        split: impl Into<String>,
        iteration: usize,
        items: Vec<MetricValue>,
    ) {
        self.handle.log_metric(epoch, split, iteration, items);
    }

    /// Log a metric definition.
    pub fn log_metric_definition(&self, spec: MetricSpec) {
        self.handle.log_metric_definition(spec);
    }

    /// Log aggregated metric values for an epoch and split.
    pub fn log_epoch_summary(
        &self,
        epoch: usize,
        split: impl Into<String>,
        items: Vec<MetricValue>,
    ) {
        self.handle.log_epoch_summary(epoch, split, items);
    }

    /// Log scalar summary values without an epoch axis.
    pub fn log_summary(&self, items: Vec<MetricValue>) {
        self.handle.log_summary(items);
    }

    /// Encode and persist an artifact.
    pub fn save_artifact<E: BundleEncode>(
        &self,
        name: impl AsRef<str>,
        kind: ArtifactKind,
        artifact: E,
        settings: &E::Settings,
    ) -> Result<(), ExperimentError> {
        self.handle.save_artifact(name, kind, artifact, settings)
    }

    /// Load and decode an artifact from a compatible experiment identifier.
    pub fn use_artifact<D: BundleDecode>(
        &self,
        experiment_id: impl Into<ExperimentId>,
        name: impl AsRef<str>,
        settings: &D::Settings,
    ) -> Result<D, ExperimentError> {
        self.handle.use_artifact(experiment_id, name, settings)
    }

    /// Create a child activity builder.
    pub fn activity(&self, name: impl Into<String>) -> ActivityBuilder {
        self.handle.activity(name)
    }

    /// Clone a lightweight [`ExperimentRunHandle`] for async tasks, worker threads, or adapter
    /// objects that should not own run finalization.
    pub fn handle(&self) -> ExperimentRunHandle {
        self.handle.clone()
    }
}

impl ExperimentRunHandle {
    /// Borrow the identifier of the run this handle points to.
    pub fn id(&self) -> &ExperimentId {
        &self.metadata.id
    }

    /// Return a cancellation token that can be linked to child work.
    ///
    /// Cancelling the token does not finish the run; it only broadcasts cancellation to linked
    /// tasks and adapters.
    pub fn cancel_token(&self) -> CancelToken {
        self.context_cancel_token.clone()
    }

    /// Return a handle whose logs inherit an additional scope attribute.
    ///
    /// The returned handle shares the run; only its inherited scope differs. Call-site attributes
    /// still take precedence over inherited ones.
    #[must_use]
    pub fn with_attr(&self, key: impl Into<String>, value: impl Into<serde_json::Value>) -> Self {
        let mut scope = (*self.scope).clone();
        scope.insert(key.into(), value.into());
        Self {
            scope: Arc::new(scope),
            ..self.clone()
        }
    }

    /// Return a handle whose logs inherit several additional scope attributes.
    #[must_use]
    pub fn with_attrs(&self, attrs: impl IntoIterator<Item = (String, serde_json::Value)>) -> Self {
        let mut scope = (*self.scope).clone();
        scope.extend(attrs);
        Self {
            scope: Arc::new(scope),
            ..self.clone()
        }
    }

    /// Return whether the underlying run is still accepting telemetry.
    ///
    /// Telemetry emitted against an inactive run is discarded rather than reported as an error, so
    /// use this when you need to branch on liveness.
    pub fn is_active(&self) -> bool {
        self.upgrade()
            .is_ok_and(|inner| inner.ensure_active().is_ok())
    }

    /// Log a `trace`-level message.
    pub fn log_trace(&self, message: impl Into<String>) {
        self.log(LogRecord::trace(message));
    }

    /// Log a `debug`-level message.
    pub fn log_debug(&self, message: impl Into<String>) {
        self.log(LogRecord::debug(message));
    }

    /// Log an `info`-level message.
    pub fn log_info(&self, message: impl Into<String>) {
        self.log(LogRecord::info(message));
    }

    /// Log a `warn`-level message.
    pub fn log_warn(&self, message: impl Into<String>) {
        self.log(LogRecord::warn(message));
    }

    /// Log an `error`-level message.
    pub fn log_error(&self, message: impl Into<String>) {
        self.log(LogRecord::error(message));
    }

    /// Block, briefly and best effort, until everything recorded has left the
    /// process.
    pub fn flush(&self) {
        if let Some(inner) = self.inner.upgrade() {
            let _ = inner.session.flush();
        }
    }

    pub(crate) fn for_activity(&self, activity: ActivityId, cancel_token: CancelToken) -> Self {
        Self {
            activity: Some(activity),
            context_cancel_token: cancel_token,
            ..self.clone()
        }
    }

    /// Record a structured log entry.
    pub fn log(&self, mut record: LogRecord) {
        if !self.scope.is_empty() {
            record.inherit_attrs(&self.scope);
        }
        self.emit(Event::Log {
            record,
            activity: self.activity,
        });
    }

    /// Log a named configuration object.
    ///
    /// Fails only if `config` cannot be serialized.
    pub fn log_config<C: Serialize>(
        &self,
        name: impl Into<String>,
        config: &C,
    ) -> Result<(), ExperimentError> {
        let value = serde_json::to_value(config).map_err(|error| {
            ExperimentError::with_source(
                ExperimentErrorKind::Artifact,
                "Failed to serialize experiment config",
                error,
            )
        })?;

        self.emit(Event::Config {
            name: name.into(),
            value,
        });
        Ok(())
    }

    /// Log metric values for an epoch, split, and iteration.
    pub fn log_metric(
        &self,
        epoch: usize,
        split: impl Into<String>,
        iteration: usize,
        items: Vec<MetricValue>,
    ) {
        self.emit(Event::Metrics {
            epoch,
            split: split.into(),
            iteration,
            items,
            activity: self.activity,
        });
    }

    /// Log a metric definition.
    pub fn log_metric_definition(&self, spec: MetricSpec) {
        self.emit(Event::MetricDefinition(spec));
    }

    /// Log aggregated metric values for an epoch and split.
    pub fn log_epoch_summary(
        &self,
        epoch: usize,
        split: impl Into<String>,
        items: Vec<MetricValue>,
    ) {
        self.emit(Event::EpochSummary {
            epoch,
            split: split.into(),
            items,
            activity: self.activity,
        });
    }

    /// Log scalar summary values without an epoch axis.
    pub fn log_summary(&self, items: Vec<MetricValue>) {
        self.emit(Event::Summary {
            items,
            activity: self.activity,
        });
    }

    /// Encode and persist an artifact.
    ///
    /// Artifacts are run-scoped: they are addressed by name within the experiment and the
    /// emitting stage is not recorded, so saving through an activity is equivalent to saving
    /// at the run root.
    pub fn save_artifact<E: BundleEncode>(
        &self,
        name: impl AsRef<str>,
        kind: ArtifactKind,
        artifact: E,
        settings: &E::Settings,
    ) -> Result<(), ExperimentError> {
        let inner = self.upgrade()?;
        inner.ensure_active()?;

        let artifact_fn = |bundle: &mut FsBundle| {
            artifact.encode(bundle, settings).map_err(|error| {
                ExperimentError::with_source(
                    ExperimentErrorKind::Artifact,
                    "Failed to encode artifact into bundle",
                    error,
                )
            })
        };

        inner
            .session
            .save_artifact(name.as_ref(), kind, Box::new(artifact_fn))
    }

    /// Load and decode an artifact from a compatible experiment identifier.
    pub fn use_artifact<D: BundleDecode>(
        &self,
        experiment_id: impl Into<ExperimentId>,
        name: impl AsRef<str>,
        settings: &D::Settings,
    ) -> Result<D, ExperimentError> {
        let inner = self.upgrade()?;
        inner.ensure_active()?;
        let name = name.as_ref();
        let experiment_id = experiment_id.into();
        let artifact = inner
            .reader
            .load_artifact_raw(experiment_id.clone(), name)
            .map_err(|error| {
                ExperimentError::with_source(
                    ExperimentErrorKind::Artifact,
                    format!("Failed to load artifact bundle for {name}"),
                    error,
                )
            })?;

        let decoded = D::decode(&artifact.bundle, settings).map_err(|error| {
            ExperimentError::with_source(
                ExperimentErrorKind::Artifact,
                format!("Failed to decode artifact: {name}"),
                error,
            )
        })?;

        self.emit(Event::ArtifactUsed {
            experiment_id,
            reference: artifact.reference,
        });

        Ok(decoded)
    }

    /// Create a child activity builder.
    pub fn activity(&self, name: impl Into<String>) -> ActivityBuilder {
        let inner = match self.upgrade() {
            Ok(inner) => inner,
            Err(_) => {
                return ActivityBuilder::detached(
                    name,
                    self.activity,
                    self.context_cancel_token.clone(),
                    self.clone(),
                );
            }
        };

        ActivityBuilder::new(
            inner.activity_id_allocator.clone(),
            self.control.clone(),
            name,
            self.clone(),
        )
        .with_parent(self.activity, self.context_cancel_token.clone())
    }

    /// Hand a telemetry event to the backend session.
    ///
    /// Telemetry is fire-and-forget: an event raised against a dropped or finished run is
    /// discarded. Operations whose failure the caller can act on — artifact encoding, artifact
    /// storage, argument and config serialization — report errors through their own return types
    /// instead.
    fn emit(&self, event: Event) {
        let Ok(inner) = self.upgrade() else {
            return;
        };
        if inner.ensure_active().is_err() {
            return;
        }

        inner.session.record_event(event).ok();
    }

    fn upgrade(&self) -> Result<Arc<RunInner>, ExperimentError> {
        self.inner.upgrade().ok_or(ExperimentError::new(
            ExperimentErrorKind::InactiveRun,
            "Experiment run is no longer active",
        ))
    }
}

impl RunInner {
    fn ensure_active(&self) -> Result<(), ExperimentError> {
        let state = self.state.lock().unwrap();
        match *state {
            RunState::Active => Ok(()),
            RunState::Finished => Err(ExperimentError::new(
                ExperimentErrorKind::AlreadyFinished,
                "Experiment run has already finished",
            )),
        }
    }

    fn finish_once(&self, completion: ExperimentCompletion) -> Result<(), ExperimentError> {
        let mut state = self.state.lock().unwrap();
        match *state {
            RunState::Finished => Err(ExperimentError::new(
                ExperimentErrorKind::AlreadyFinished,
                "Experiment run has already finished",
            )),
            RunState::Active => {
                *state = RunState::Finished;
                drop(state);
                self.session.finish(completion)
            }
        }
    }
}

/// Finalize the run on drop if it has not already been completed.
impl Drop for ExperimentRun {
    fn drop(&mut self) {
        let completion = if std::thread::panicking() {
            let reason =
                panic_watch::take_thread_panic().unwrap_or_else(|| "the run panicked".to_string());
            ExperimentCompletion::Failed(reason)
        } else if self.inner.control.is_run_cancelled() {
            ExperimentCompletion::Cancelled
        } else {
            ExperimentCompletion::Success
        };

        let _ = self.inner.finish_once(completion);
    }
}

#[cfg(test)]
mod tests {
    use crate::activity::ActivityEvent;
    use crate::test_support::{MockSession, create_run};

    use super::*;

    #[test]
    fn panic_unwinding_a_guard_fails_the_activity_with_the_panic() {
        let session = Arc::new(MockSession::default());
        let run_session = session.clone();

        let outcome = std::thread::Builder::new()
            .name("stage-runner".into())
            .spawn(move || {
                let run = create_run(run_session);
                let _stage = run.activity("Train fold 3 of 6").cancellable().start();
                panic!("fusion stream out of order");
            })
            .unwrap()
            .join();
        assert!(outcome.is_err());

        let finished = session
            .activity_events()
            .into_iter()
            .find_map(|event| match event {
                ActivityEvent::Finished {
                    status, message, ..
                } => Some((status, message)),
                _ => None,
            })
            .expect("the stage reported an ending");
        assert!(matches!(finished.0, ActivityStatus::Failed));
        let message = finished.1.expect("the ending carried the panic");
        assert!(message.contains("fusion stream out of order"), "{message}");

        let completions = session.completions.lock().unwrap();
        match completions.as_slice() {
            [ExperimentCompletion::Failed(reason)] => {
                assert!(reason.contains("fusion stream out of order"), "{reason}");
            }
            other => panic!("unexpected completions: {other:?}"),
        }
    }

    #[test]
    fn panicking_run_reports_the_panic_as_its_failure_reason() {
        let session = Arc::new(MockSession::default());
        let run_session = session.clone();

        let outcome = std::thread::Builder::new()
            .name("fold-runner".into())
            .spawn(move || {
                let _run = create_run(run_session);
                panic!("kernel exploded mid-batch");
            })
            .unwrap()
            .join();
        assert!(outcome.is_err());

        let completions = session.completions.lock().unwrap();
        match completions.as_slice() {
            [ExperimentCompletion::Failed(reason)] => {
                assert!(reason.contains("kernel exploded mid-batch"), "{reason}");
                assert!(reason.contains("panicked at"), "{reason}");
                assert!(reason.contains("fold-runner"), "{reason}");
            }
            other => panic!("unexpected completions: {other:?}"),
        }
    }

    #[test]
    fn watched_run_logs_panics_from_other_threads() {
        let session = Arc::new(MockSession::default());
        let run = create_run(session.clone());
        let _watch = run.capture_panics();

        let outcome = std::thread::Builder::new()
            .name("device-runner".into())
            .spawn(|| panic!("unsupported op: plane.elect"))
            .unwrap()
            .join();
        assert!(outcome.is_err());

        let events = session.events.lock().unwrap();
        let logged = events.iter().any(|event| match event {
            Event::Log { record, .. } => {
                record.level == LogLevel::Error
                    && record.message.contains("unsupported op: plane.elect")
                    && record.message.contains("device-runner")
            }
            _ => false,
        });
        assert!(logged, "no error log carried the panic: {events:?}");
        drop(events);

        // The hook pushes what it just recorded out of the process while it
        // still can; an aborting teardown must not be able to eat it.
        assert!(
            *session.flushes.lock().unwrap() > 0,
            "the hook never flushed"
        );
    }

    #[test]
    fn run_context_records_events() {
        let session = Arc::new(MockSession::default());
        let run = create_run(session.clone());

        run.log_info("hello");

        let events = session.events.lock().unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::Log { record, .. } => assert_eq!(record.message, "hello"),
            event => panic!("unexpected event: {event:?}"),
        }
    }

    #[test]
    fn telemetry_records_the_emitting_activity_scope() {
        let session = Arc::new(MockSession::default());
        let run = create_run(session.clone());
        let activity = run.activity("cross-validation").start();
        let items = || {
            vec![MetricValue {
                name: "mean_score".to_string(),
                value: 0.8,
            }]
        };

        activity.log_info("in scope");
        activity.log_metric(1, "train", 1, items());
        activity.log_epoch_summary(1, "train", items());
        activity.log_summary(items());
        run.log_info("out of scope");

        let events = session.events.lock().unwrap();
        let event_activity = |event: &Event| match event {
            Event::Log { activity, .. }
            | Event::Metrics { activity, .. }
            | Event::EpochSummary { activity, .. }
            | Event::Summary { activity, .. } => *activity,
            event => panic!("unexpected event: {event:?}"),
        };
        assert!(
            events[1..=4]
                .iter()
                .all(|event| event_activity(event) == Some(activity.id()))
        );
        assert_eq!(event_activity(&events[5]), None);
    }

    #[test]
    fn activity_run_finishes_fails_and_propagates_panics() {
        let session = Arc::new(MockSession::default());
        let run = create_run(session.clone());

        let value = run
            .activity("ok")
            .run(|_| Ok::<_, &'static str>(7))
            .unwrap();
        assert_eq!(value, 7);

        let error = run
            .activity("error")
            .run(|_| Err::<(), _>("fold failed"))
            .unwrap_err();
        assert_eq!(error, "fold failed");

        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _: Result<(), &'static str> = run.activity("panic").run(|_| panic!("boom"));
        }));
        assert!(panic.is_err());

        let events = session.events.lock().unwrap();
        let completions = events
            .iter()
            .filter_map(|event| match event {
                Event::Activity(ActivityEvent::Finished {
                    status, message, ..
                }) => Some((status, message.as_deref())),
                _ => None,
            })
            .collect::<Vec<_>>();
        match completions.as_slice() {
            [
                (ActivityStatus::Success, None),
                (ActivityStatus::Failed, Some("fold failed")),
                (ActivityStatus::Failed, Some(panicked)),
            ] => assert!(panicked.contains("boom"), "{panicked}"),
            other => panic!("unexpected completions: {other:?}"),
        }
    }

    #[test]
    fn child_activity_from_reference_uses_reference_as_parent() {
        let session = Arc::new(MockSession::default());
        let run = create_run(session.clone());
        let parent = run.activity("parent").start();
        let activity = parent.share();

        let _child = activity.activity("child").start();

        let events = session.events.lock().unwrap();
        let Event::Activity(ActivityEvent::Started { activity: child }) = &events[1] else {
            panic!("unexpected event: {:?}", events[1]);
        };
        assert_eq!(child.parent, Some(parent.id()));
    }

    #[test]
    fn activity_reference_preserves_inherited_log_attributes() {
        let session = Arc::new(MockSession::default());
        let run = create_run(session.clone());
        let guard = run
            .handle()
            .with_attr("pipeline", "cv")
            .activity("fold")
            .start();
        let activity = guard.share().with_attr("fold", 1i64);

        activity.log(LogRecord::info("started").with("fold", 2i64));

        let events = session.events.lock().unwrap();
        let Event::Log {
            record,
            activity: Some(id),
        } = &events[1]
        else {
            panic!("unexpected event: {:?}", events[1]);
        };
        assert_eq!(*id, guard.id());
        assert_eq!(
            record.attributes.get("pipeline"),
            Some(&serde_json::json!("cv"))
        );
        assert_eq!(record.attributes.get("fold"), Some(&serde_json::json!(2)));
    }

    #[test]
    fn activity_reference_discards_telemetry_after_the_run_is_dropped() {
        let session = Arc::new(MockSession::default());
        let activity = {
            let run = create_run(session.clone());
            let guard = run.activity("fold").start();
            let activity = guard.share();
            guard.finish();
            activity
        };
        let recorded = session.events.lock().unwrap().len();

        assert!(!activity.is_active());
        activity.log_info("late");

        assert_eq!(session.events.lock().unwrap().len(), recorded);
    }

    #[test]
    fn scoped_handle_inherits_attributes_and_call_site_wins() {
        let session = Arc::new(MockSession::default());
        let run = create_run(session.clone());

        let scoped = run
            .handle()
            .with_attr("phase", "train")
            .with_attr("shard", 1i64);
        scoped.log(
            LogRecord::warn("slow step")
                .with("shard", 2i64)
                .with("elapsed_ms", 900i64),
        );

        let events = session.events.lock().unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::Log { record, .. } => {
                assert_eq!(record.level, LogLevel::Warn);
                assert_eq!(record.message, "slow step");
                // Inherited from scope.
                assert_eq!(
                    record.attributes.get("phase").and_then(|v| v.as_str()),
                    Some("train")
                );
                // Call-site value overrides the inherited scope value.
                assert_eq!(
                    record.attributes.get("shard").and_then(|v| v.as_i64()),
                    Some(2)
                );
                assert_eq!(
                    record.attributes.get("elapsed_ms").and_then(|v| v.as_i64()),
                    Some(900)
                );
            }
            event => panic!("unexpected event: {event:?}"),
        }
    }

    #[test]
    fn finish_marks_handle_inactive_and_discards_later_telemetry() {
        let session = Arc::new(MockSession::default());
        let run = create_run(session.clone());
        let handle = run.handle();

        run.finish().unwrap();

        assert!(!handle.is_active());
        handle.log_info("after-finish");
        assert!(session.events.lock().unwrap().is_empty());
    }

    #[test]
    fn drop_marks_run_as_finished_successfully() {
        let session = Arc::new(MockSession::default());

        {
            let _run = create_run(session.clone());
        }

        let completions = session.completions.lock().unwrap();
        assert_eq!(completions.as_slice(), &[ExperimentCompletion::Success]);
    }

    #[test]
    fn cancel_marks_run_cancelled_on_drop() {
        let session = Arc::new(MockSession::default());

        {
            let run = create_run(session.clone());
            run.cancel().unwrap();
        }

        let completions = session.completions.lock().unwrap();
        assert_eq!(completions.as_slice(), &[ExperimentCompletion::Cancelled]);
    }

    #[test]
    fn cancel_does_not_prevent_logging_before_drop() {
        let session = Arc::new(MockSession::default());
        let run = create_run(session.clone());

        run.cancel().unwrap();
        run.log_info("still-logging");

        let events = session.events.lock().unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::Log { record, .. } => assert_eq!(record.message, "still-logging"),
            event => panic!("unexpected event: {event:?}"),
        }
    }

    #[test]
    fn dropped_run_handle_activity_records_no_event() {
        let session = Arc::new(MockSession::default());
        let run = create_run(session.clone());
        let handle = run.handle();
        drop(run);

        let _progress = handle.activity("late").meter(1, "items").start();

        let events = session.events.lock().unwrap();
        assert!(events.is_empty());
    }
}
