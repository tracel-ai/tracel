//! Experiment tracking primitives.
//!
//! The core APIs are:
//! - [`ExperimentRun`], which owns the lifecycle of an active experiment.
//! - [`ExperimentRunHandle`], which is a lightweight cloneable view for logging and artifact access
//!   from background tasks or other threads.
//! - [`ExperimentContext`], the telemetry surface shared by runs, handles, activity guards, and
//!   [`Activity`] references.
//!
//! Optional capabilities are exposed through extension traits:
//! - [`ExperimentRunHandleExt`] for cloning a shareable handle.
//! - [`ExperimentGlobalExt`] for ambient thread-local experiment context.
//! - [`integration::training::ExperimentTrainingExt`] for Burn `train` adapters.
//! - [`integration::training::SupervisedTrainingExperimentExt`] for one-line Burn builder wiring.
//! - [`integration::tracing::ExperimentTracingExt`] for tracing span helpers.
//!
//! Backends are connected through the [`ExperimentProvider`] port. [`ExperimentModule`] and
//! [`ExperimentJob`] are the user-facing entry points for running a job and logging its result.

use std::fmt;
use std::str::FromStr;
use std::sync::{Arc, Mutex, Weak};

use serde::Serialize;

mod activity;
mod cancellation;
mod context;
mod control;
mod log;
mod provider;
pub mod reader;
mod scope;
pub mod session;
#[cfg(test)]
mod test_support;

pub mod error;
pub mod integration;

pub use activity::{
    ActivityBuilder, ActivityEvent, ActivityGuard, ActivityId, ActivityMeter, ActivitySpec,
    ActivityStatus,
};
pub use cancellation::{CancelToken, Cancellable};
pub use context::{
    CurrentExperimentGuard, ExperimentGlobalExt, ExperimentInstrument, WithCurrentExperiment,
};
pub use control::ExperimentRunControl;
pub use log::{LogLevel, LogRecord};
pub use provider::{ExperimentFn, ExperimentJob, ExperimentModule, ExperimentProvider};
pub use scope::{Activity, ExperimentContext};

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
/// Use [`ExperimentRunHandleExt::handle`] when you need to share logging and artifact access without
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
/// If the originating [`ExperimentRun`] is finished or dropped, existing handles become inactive
/// and will reject further operations.
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
}

impl From<&ExperimentRun> for ExperimentRunHandle {
    fn from(value: &ExperimentRun) -> Self {
        value.handle()
    }
}

impl ExperimentRun {
    /// Borrow the identifier for the underlying run.
    pub fn id(&self) -> &ExperimentId {
        self.handle.id()
    }

    /// Log the serialized input arguments for the run.
    pub fn log_args<A: Serialize>(&self, args: &A) -> Result<(), ExperimentError> {
        let value = serde_json::to_value(args).map_err(|error| {
            ExperimentError::with_source(
                ExperimentErrorKind::Artifact,
                "Failed to serialize experiment arguments",
                error,
            )
        })?;

        self.handle.record_event(Event::Args(value))
    }
}

/// Extension trait for cloning shareable handles from an [`ExperimentRun`].
///
/// Import this trait when you want a lightweight [`ExperimentRunHandle`] for async tasks, worker
/// threads, or adapter objects that should not own run finalization.
///
/// # Example
///
/// ```ignore
/// use tracel_experiment::{ExperimentContext, ExperimentRun, ExperimentRunHandleExt};
///
/// let run = ExperimentRun::local("./runs").unwrap();
/// let handle = run.handle();
///
/// std::thread::spawn(move || {
///     let _ = handle.log_info("worker started");
/// });
/// ```
pub trait ExperimentRunHandleExt {
    /// Clone a handle that can be shared across tasks and threads.
    fn handle(&self) -> ExperimentRunHandle;
}

impl ExperimentRunHandleExt for ExperimentRun {
    fn handle(&self) -> ExperimentRunHandle {
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
}

impl ExperimentRunHandle {
    fn record_event(&self, event: Event) -> Result<(), ExperimentError> {
        let inner = self.upgrade()?;
        inner.ensure_active()?;
        inner.session.record_event(event)
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
        let completion = if self.inner.control.is_run_cancelled() {
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
    use tracel_artifact::bundle::{BundleEncode, BundleSink};

    use super::*;

    #[test]
    fn run_context_records_events() {
        let session = Arc::new(MockSession::default());
        let run = create_run(session.clone());

        run.log_info("hello").unwrap();

        let events = session.events.lock().unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::Log { record, .. } => assert_eq!(record.message, "hello"),
            event => panic!("unexpected event: {event:?}"),
        }
    }

    #[test]
    fn metrics_record_the_emitting_activity_scope() {
        let session = Arc::new(MockSession::default());
        let run = create_run(session.clone());
        let activity = run.activity("train").start();

        activity
            .log_metric(
                1,
                "train",
                1,
                vec![MetricValue {
                    name: "loss".to_string(),
                    value: 0.25,
                }],
            )
            .unwrap();
        run.log_metric(
            1,
            "valid",
            1,
            vec![MetricValue {
                name: "loss".to_string(),
                value: 0.2,
            }],
        )
        .unwrap();

        let events = session.events.lock().unwrap();
        assert!(matches!(
            &events[1],
            Event::Metrics {
                activity: Some(id),
                ..
            } if *id == activity.id()
        ));
        assert!(matches!(&events[2], Event::Metrics { activity: None, .. }));
    }

    #[test]
    fn summaries_record_the_emitting_activity_scope() {
        let session = Arc::new(MockSession::default());
        let run = create_run(session.clone());
        let activity = run.activity("cross-validation").start();
        let items = || {
            vec![MetricValue {
                name: "mean_score".to_string(),
                value: 0.8,
            }]
        };

        activity.log_summary(items()).unwrap();
        run.log_summary(items()).unwrap();

        let events = session.events.lock().unwrap();
        assert!(matches!(
            &events[1],
            Event::Summary {
                activity: Some(id),
                ..
            } if *id == activity.id()
        ));
        assert!(matches!(&events[2], Event::Summary { activity: None, .. }));
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
        assert!(matches!(
            completions.as_slice(),
            [
                (ActivityStatus::Success, None),
                (ActivityStatus::Abandoned, Some("fold failed")),
                (ActivityStatus::Abandoned, None),
            ]
        ));
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

        activity
            .log(LogRecord::info("started").with("fold", 2i64))
            .unwrap();

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
    fn activity_reference_becomes_inactive_after_the_run_is_dropped() {
        let session = Arc::new(MockSession::default());
        let activity = {
            let run = create_run(session);
            let guard = run.activity("fold").start();
            let activity = guard.share();
            guard.finish();
            activity
        };

        let error = activity.log_info("late").unwrap_err();
        assert_eq!(error.kind, ExperimentErrorKind::InactiveRun);
    }

    #[test]
    fn artifacts_record_the_emitting_activity_scope() {
        struct EmptyArtifact;

        impl BundleEncode for EmptyArtifact {
            type Settings = ();
            type Error = String;

            fn encode<O: BundleSink>(
                self,
                _sink: &mut O,
                _settings: &Self::Settings,
            ) -> Result<(), Self::Error> {
                Ok(())
            }
        }

        let session = Arc::new(MockSession::default());
        let run = create_run(session.clone());
        let activity = run.activity("fold").start();

        activity
            .save_artifact("fold-model", ArtifactKind::Model, EmptyArtifact, &())
            .unwrap();
        run.save_artifact("model", ArtifactKind::Model, EmptyArtifact, &())
            .unwrap();

        let activities = session.artifact_activities.lock().unwrap();
        assert_eq!(activities.as_slice(), &[Some(activity.id()), None]);
    }

    #[test]
    fn activity_reference_is_shareable_and_static() {
        fn assert_bounds<T: Clone + Send + Sync + 'static>() {}
        assert_bounds::<Activity>();
    }

    #[test]
    fn level_methods_record_the_matching_level() {
        let session = Arc::new(MockSession::default());
        let run = create_run(session.clone());

        run.log_trace("t").unwrap();
        run.log_debug("d").unwrap();
        run.log_info("i").unwrap();
        run.log_warn("w").unwrap();
        run.log_error("e").unwrap();

        let events = session.events.lock().unwrap();
        let levels: Vec<_> = events
            .iter()
            .map(|event| match event {
                Event::Log { record, .. } => record.level,
                event => panic!("unexpected event: {event:?}"),
            })
            .collect();
        assert_eq!(
            levels,
            vec![
                LogLevel::Trace,
                LogLevel::Debug,
                LogLevel::Info,
                LogLevel::Warn,
                LogLevel::Error,
            ]
        );
    }

    #[test]
    fn scoped_handle_inherits_attributes_and_call_site_wins() {
        let session = Arc::new(MockSession::default());
        let run = create_run(session.clone());

        let scoped = run
            .handle()
            .with_attr("phase", "train")
            .with_attr("shard", 1i64);
        scoped
            .log(
                LogRecord::warn("slow step")
                    .with("shard", 2i64)
                    .with("elapsed_ms", 900i64),
            )
            .unwrap();

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
    fn finish_marks_handle_inactive() {
        let session = Arc::new(MockSession::default());
        let run = create_run(session.clone());
        let handle = run.handle();

        run.finish().unwrap();

        let err = handle.log_info("after-finish").unwrap_err();
        assert_eq!(err.kind, ExperimentErrorKind::InactiveRun);
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
        run.log_info("still-logging").unwrap();

        let events = session.events.lock().unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::Log { record, .. } => assert_eq!(record.message, "still-logging"),
            event => panic!("unexpected event: {event:?}"),
        }
    }

    #[test]
    fn run_metered_activity_start_records_progress_event() {
        let session = Arc::new(MockSession::default());
        let run = create_run(session.clone());

        let _progress = run.activity("load").meter(1, "items").start();

        let events = session.events.lock().unwrap();
        assert!(matches!(
            events.as_slice(),
            [Event::Activity(ActivityEvent::Started { activity })] if activity.name == "load"
        ));
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
