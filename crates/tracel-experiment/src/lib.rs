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
//! - `integration::training::ExperimentTrainingExt` for Burn `train` adapters (behind the `burn`
//!   feature).
//! - `integration::training::SupervisedTrainingExperimentExt` for one-line Burn builder wiring
//!   (behind the `burn` feature).
//! - [`integration::tracing::ExperimentTracingExt`] for tracing span helpers.
//!
//! Backends are connected through the [`ExperimentProvider`] port. [`Experiments`] is the
//! user-facing entry point: [`Experiments::run`] runs code in a new experiment and ends it by what
//! the code returns, and [`Experiments::start`] starts one and returns its run. With the `local`
//! feature, `local::LocalExperiments` records experiments on this machine.

use std::fmt;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

use serde::Serialize;
use tracel_artifact::bundle::{BundleDecode, BundleEncode, FsBundle};

mod activity;
mod cancellation;
mod context;
mod control;
#[cfg(feature = "local")]
pub mod local;
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
pub use provider::{ExperimentProvider, Experiments};

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
/// events, persist artifacts, and expose a cancellation token to child work. When that token is
/// cancelled while the run is active, the run logs a warning that a stop was requested, once.
///
/// Use [`ExperimentRun::handle`] when you need to share logging and artifact access without
/// transferring lifecycle ownership.
///
/// A run ends as completed only through [`Self::finish`]. One dropped before [`Self::finish`] or
/// [`Self::fail`] ends as failed: with the panic's message while a panic unwinds it, and otherwise
/// because nothing said it succeeded, as when an error returns early past it.
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
    /// This is the low-level constructor an [`ExperimentProvider`] uses. Most callers start a run
    /// with [`Experiments::start`] or [`Experiments::run`] instead.
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
        control.cancel_token().link(StopRequestLog {
            run: Arc::downgrade(&inner),
            requested: AtomicBool::new(false),
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
    /// Cancelling the token does not finish the run: it broadcasts cancellation to linked tasks
    /// and adapters, and the run logs, once, that a stop was requested.
    pub fn cancel_token(&self) -> CancelToken {
        self.inner.control.cancel_token()
    }

    /// Ask the run to stop: cancels its [`cancel_token`](Self::cancel_token).
    ///
    /// The run remains usable. It still ends as completed or failed, as [`Self::finish`],
    /// [`Self::fail`] or dropping it records.
    pub fn cancel(&self) -> Result<(), ExperimentError> {
        self.inner.ensure_active()?;
        self.inner.control.cancel_run();
        Ok(())
    }

    /// Mark the run as successful and finalize the backend session.
    ///
    /// A run dropped without calling [`Self::finish`] or [`Self::fail`] is finalized as failed.
    /// Any cloned [`ExperimentRunHandle`] becomes inactive afterwards.
    pub fn finish(self) -> Result<(), ExperimentError> {
        self.complete(ExperimentCompletion::Success)
    }

    /// Mark the run as failed and finalize the backend session.
    ///
    /// Any cloned [`ExperimentRunHandle`] becomes inactive afterwards.
    pub fn fail(self, reason: impl Into<String>) -> Result<(), ExperimentError> {
        self.complete(ExperimentCompletion::Failed(reason.into()))
    }

    /// Borrow the identifier for the underlying run.
    pub fn id(&self) -> &ExperimentId {
        self.handle.id()
    }

    /// Log the serialized input arguments for the run, such as the configuration it trains with,
    /// which the console lists as the experiment's config.
    ///
    /// A run started from [`Experiments`] given [`arguments`](Experiments::arguments) records them
    /// when it starts, so this is only needed to record different ones. The arguments logged last
    /// are the run's: logging them replaces those recorded before. Fails only if `args` cannot be
    /// serialized.
    pub fn log_args<A: Serialize>(&self, args: &A) -> Result<(), ExperimentError> {
        let value = serde_json::to_value(args).map_err(|error| {
            ExperimentError::with_source(
                ExperimentErrorKind::Artifact,
                "Failed to serialize experiment arguments",
                error,
            )
        })?;

        self.record_args(value);
        Ok(())
    }

    /// Records `args`, the run's input as JSON, as its arguments.
    fn record_args(&self, args: serde_json::Value) {
        self.handle.emit(Event::Args(args));
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

    /// Finalizes the backend session with `completion`.
    ///
    /// Fails with [`ExperimentErrorKind::AlreadyFinished`] when the run has already finished.
    fn complete(&self, completion: ExperimentCompletion) -> Result<(), ExperimentError> {
        self.inner.mark_finished()?;
        self.inner.session.finish(completion)
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
        if let Ok(inner) = self.upgrade() {
            inner.emit(event);
        }
    }

    fn upgrade(&self) -> Result<Arc<RunInner>, ExperimentError> {
        self.inner.upgrade().ok_or(ExperimentError::new(
            ExperimentErrorKind::InactiveRun,
            "Experiment run is no longer active",
        ))
    }
}

impl RunInner {
    /// Hands `event` to the backend session, or discards it once the run has finished.
    fn emit(&self, event: Event) {
        if self.ensure_active().is_ok() {
            self.session.record_event(event).ok();
        }
    }

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

    /// Moves the run to finished, once: fails when it already is.
    fn mark_finished(&self) -> Result<(), ExperimentError> {
        let mut state = self.state.lock().unwrap();
        match *state {
            RunState::Finished => Err(ExperimentError::new(
                ExperimentErrorKind::AlreadyFinished,
                "Experiment run has already finished",
            )),
            RunState::Active => {
                *state = RunState::Finished;
                Ok(())
            }
        }
    }
}

/// The warning a run logs when its cancel token is cancelled while it is active.
const STOP_REQUESTED: &str = "Stop requested; the run ends when the code recording it returns";

/// Logs [`STOP_REQUESTED`] to a run, once, when the run's cancel token is cancelled.
///
/// Linked to the run's token, it logs on the thread that cancels it. It holds the run weakly, as
/// the token can outlive the run, and logs nothing once the run has finished.
struct StopRequestLog {
    run: Weak<RunInner>,
    requested: AtomicBool,
}

impl Cancellable for StopRequestLog {
    fn cancel(&self) {
        if self.requested.swap(true, Ordering::AcqRel) {
            return;
        }
        if let Some(run) = self.run.upgrade() {
            run.emit(Event::Log {
                record: LogRecord::warn(STOP_REQUESTED),
                activity: None,
            });
        }
    }

    fn is_cancelled(&self) -> bool {
        self.requested.load(Ordering::Acquire)
    }
}

/// Why a run dropped before it was finished or failed fails, when no panic unwinds it.
const DROPPED_UNFINISHED: &str = "the run was dropped without finish() or fail()";

/// Finalize the run on drop if it has not already been completed: as failed, with the panic's
/// message while a panic unwinds it.
impl Drop for ExperimentRun {
    fn drop(&mut self) {
        let reason = if std::thread::panicking() {
            panic_watch::take_thread_panic().unwrap_or_else(|| "the run panicked".to_string())
        } else {
            DROPPED_UNFINISHED.to_string()
        };

        let _ = self.complete(ExperimentCompletion::Failed(reason));
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use serde_json::json;

    use crate::test_support::{
        MockRun, MockSession, create_run, create_run_with_control, mock_run,
    };

    use super::*;

    const PANIC: &str = "kernel exploded mid-batch";
    const REASON: &str = "interrupted";

    fn metrics() -> Vec<MetricValue> {
        vec![MetricValue {
            name: "mean_score".to_string(),
            value: 0.8,
        }]
    }

    fn recorded_activity(event: &Event) -> Option<ActivityId> {
        match event {
            Event::Log { activity, .. }
            | Event::Metrics { activity, .. }
            | Event::EpochSummary { activity, .. }
            | Event::Summary { activity, .. } => *activity,
            event => panic!("{event:?} is not telemetry"),
        }
    }

    #[test]
    fn a_run_unwound_by_a_panic_fails_with_the_panic() {
        let session = Arc::new(MockSession::default());
        let run_session = session.clone();

        std::thread::spawn(move || {
            let _run = create_run(run_session);
            panic!("{PANIC}");
        })
        .join()
        .unwrap_err();

        assert!(matches!(
            session.completions.lock().unwrap().as_slice(),
            [ExperimentCompletion::Failed(reason)] if reason.ends_with(PANIC)
        ));
    }

    #[rstest]
    #[case::log(|activity: &Activity| activity.log_info("in scope"))]
    #[case::metric(|activity: &Activity| activity.log_metric(1, "train", 1, metrics()))]
    #[case::epoch_summary(|activity: &Activity| activity.log_epoch_summary(1, "train", metrics()))]
    #[case::summary(|activity: &Activity| activity.log_summary(metrics()))]
    fn telemetry_through_an_activity_records_the_activity(
        mock_run: MockRun,
        #[case] emit: fn(&Activity),
    ) {
        let guard = mock_run.run.activity("cross-validation").start();

        emit(&guard);

        let events = mock_run.session.events.lock().unwrap();
        assert_eq!(recorded_activity(events.last().unwrap()), Some(guard.id()));
    }

    #[rstest]
    fn an_activity_started_from_another_is_its_child(mock_run: MockRun) {
        let parent = mock_run.run.activity("parent").start();

        let _child = parent.share().activity("child").start();

        let started = mock_run.session.started_activities();
        assert_eq!(started.last().unwrap().parent, Some(parent.id()));
    }

    #[rstest]
    fn an_activity_keeps_the_attributes_of_the_handle_it_started_from(mock_run: MockRun) {
        let guard = mock_run
            .run
            .handle()
            .with_attr("pipeline", "cv")
            .activity("fold")
            .start();

        guard.share().log_info("started");

        assert_eq!(
            mock_run.session.logs()[0].attributes.get("pipeline"),
            Some(&json!("cv"))
        );
    }

    #[rstest]
    fn a_log_through_a_scoped_handle_inherits_its_attributes(mock_run: MockRun) {
        let scoped = mock_run.run.handle().with_attr("phase", "train");

        scoped.log_warn("slow step");

        assert_eq!(
            mock_run.session.logs()[0].attributes.get("phase"),
            Some(&json!("train"))
        );
    }

    #[rstest]
    #[case::finished(|run: ExperimentRun| run.finish().unwrap())]
    #[case::dropped(drop)]
    fn a_handle_of_a_run_that_ended_is_inactive(mock_run: MockRun, #[case] end: fn(ExperimentRun)) {
        let handle = mock_run.run.handle();

        end(mock_run.run);

        assert!(!handle.is_active());
    }

    #[rstest]
    #[case::finished(|run: ExperimentRun| run.finish().unwrap())]
    #[case::dropped(drop)]
    fn a_log_through_a_handle_of_a_run_that_ended_is_discarded(
        mock_run: MockRun,
        #[case] end: fn(ExperimentRun),
    ) {
        let handle = mock_run.run.handle();
        end(mock_run.run);

        handle.log_info("late");

        assert!(mock_run.session.logs().is_empty());
    }

    #[rstest]
    fn an_activity_started_from_a_handle_of_a_dropped_run_records_nothing(mock_run: MockRun) {
        let handle = mock_run.run.handle();
        drop(mock_run.run);

        let _late = handle.activity("late").meter(1, "items").start();

        assert!(mock_run.session.events.lock().unwrap().is_empty());
    }

    #[rstest]
    fn a_run_dropped_before_it_finishes_fails(mock_run: MockRun) {
        drop(mock_run.run);

        assert!(matches!(
            mock_run.session.completions.lock().unwrap().as_slice(),
            [ExperimentCompletion::Failed(_)]
        ));
    }

    #[rstest]
    fn a_finished_run_ends_once_as_completed(mock_run: MockRun) {
        mock_run.run.finish().unwrap();

        assert_eq!(
            *mock_run.session.completions.lock().unwrap(),
            [ExperimentCompletion::Success]
        );
    }

    #[rstest]
    #[case::finished(|run: ExperimentRun| run.finish().unwrap(), ExperimentCompletion::Success)]
    #[case::failed(
        |run: ExperimentRun| run.fail(REASON).unwrap(),
        ExperimentCompletion::Failed(REASON.to_string())
    )]
    fn a_cancelled_run_ends_as_it_is_ended(
        mock_run: MockRun,
        #[case] end: fn(ExperimentRun),
        #[case] completion: ExperimentCompletion,
    ) {
        mock_run.run.cancel().unwrap();

        end(mock_run.run);

        assert_eq!(*mock_run.session.completions.lock().unwrap(), [completion]);
    }

    #[rstest]
    fn a_cancelled_run_keeps_logging_after_the_stop_warning(mock_run: MockRun) {
        mock_run.run.cancel().unwrap();

        mock_run.run.log_info("still logging");

        assert_eq!(
            mock_run.session.log_messages(),
            [STOP_REQUESTED, "still logging"]
        );
    }

    #[rstest]
    fn cancelling_an_activity_logs_no_stop_request(mock_run: MockRun) {
        let activity = mock_run.run.activity("epoch 1").cancellable().start();

        activity.cancel_token().cancel();

        assert!(mock_run.session.logs().is_empty());
    }

    #[rstest]
    fn a_stop_request_is_logged_once_however_often_the_run_is_cancelled(mock_run: MockRun) {
        mock_run.run.cancel_token().cancel();
        mock_run.run.cancel().unwrap();
        mock_run.run.cancel_token().cancel();

        assert_eq!(mock_run.session.log_messages(), [STOP_REQUESTED]);
    }

    #[test]
    fn a_run_whose_token_is_already_cancelled_logs_the_stop_request_when_it_starts() {
        let session = Arc::new(MockSession::default());
        let control = ExperimentRunControl::default();
        control.cancel_run();

        let _run = create_run_with_control(session.clone(), control);

        assert_eq!(session.log_messages(), [STOP_REQUESTED]);
    }

    #[rstest]
    fn a_stop_requested_once_the_run_has_finished_is_not_logged(mock_run: MockRun) {
        let cancel_token = mock_run.run.cancel_token();
        mock_run.run.finish().unwrap();

        cancel_token.cancel();

        assert!(mock_run.session.logs().is_empty());
    }
}
