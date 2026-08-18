//! Activity tracking primitives for experiment runs.
//!
//! Progress is modeled as a tree of named activities. Starting an activity emits a
//! [`ActivityEvent::Started`] event, numeric updates emit [`ActivityEvent::Updated`],
//! and explicit or drop-based completion emits [`ActivityEvent::Finished`].

use std::{
    num::NonZeroU64,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use serde::Serialize;
use tracel_artifact::bundle::{BundleDecode, BundleEncode};

use crate::cancellation::CancelToken;
use crate::context::CurrentExperimentGuard;
use crate::control::ExperimentRunControl;
use crate::error::ExperimentError;
use crate::session::Event;
use crate::{ArtifactKind, ExperimentId, ExperimentRunHandle, LogRecord, MetricSpec, MetricValue};

/// Opaque non-zero identifier for an activity.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq, Hash)]
pub struct ActivityId(NonZeroU64);

impl ActivityId {
    /// Create an identifier from a non-zero numeric value.
    pub fn new(id: NonZeroU64) -> Self {
        Self(id)
    }

    /// Return the underlying numeric identifier.
    pub fn as_u64(self) -> u64 {
        self.0.get()
    }
}

/// Numeric meter definition for an activity.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ActivityMeter {
    /// Optional unit for progress values, such as `steps` or `bytes`.
    pub unit: Option<String>,
    /// Optional expected total for the activity.
    pub total: Option<u64>,
}

/// Metadata describing an activity when it starts.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ActivitySpec {
    /// Unique identifier for this activity.
    pub id: ActivityId,
    /// Parent activity identifier, when this activity is nested under another activity.
    pub parent: Option<ActivityId>,
    /// Human-readable activity name.
    pub name: String,
    /// Whether this activity can be cancelled by a remote controller.
    #[serde(default)]
    pub cancellable: bool,
    /// Numeric meter definition, when this activity has its own meter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meter: Option<ActivityMeter>,
    /// Extra structured metadata attached by the caller.
    #[serde(default)]
    pub attributes: serde_json::Map<String, serde_json::Value>,
}

/// Terminal state for an activity.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum ActivityStatus {
    /// The activity completed successfully.
    Success,
    /// The activity stopped before successful completion.
    Abandoned,
    /// The activity ended in an error.
    Failed,
}

/// Event emitted by an activity.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type")]
pub enum ActivityEvent {
    /// An activity was started.
    Started {
        /// The started activity metadata.
        activity: ActivitySpec,
    },
    /// An activity's numeric progress changed.
    Updated {
        /// The updated activity identifier.
        id: ActivityId,
        /// The current progress value.
        current: u64,
    },
    /// An activity emitted a human-readable message.
    Message {
        /// The activity identifier.
        id: ActivityId,
        /// The message text.
        message: String,
    },
    /// An activity reached a terminal state.
    Finished {
        /// The finished activity identifier.
        id: ActivityId,
        /// The terminal status.
        status: ActivityStatus,
        /// Optional completion message.
        message: Option<String>,
    },
}

/// Lock-free activity identifier allocator.
#[derive(Debug)]
pub(crate) struct AtomicActivityIdAllocator {
    next: AtomicU64,
}

impl AtomicActivityIdAllocator {
    /// Create an allocator that starts at identifier `1`.
    pub(crate) fn new() -> Self {
        Self {
            next: AtomicU64::new(1),
        }
    }

    /// Return the next identifier.
    pub(crate) fn next_id(&self) -> ActivityId {
        let id = self.next.fetch_add(1, Ordering::Relaxed);

        // Starts at 1, so this should only fail after overflow or wraparound.
        let id = NonZeroU64::new(id).expect("activity id allocator overflowed or produced zero");

        ActivityId(id)
    }
}

impl Default for AtomicActivityIdAllocator {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder used to configure and start an activity.
pub struct ActivityBuilder {
    id_allocator: Arc<AtomicActivityIdAllocator>,
    control: ExperimentRunControl,
    cancellation_parent: CancelToken,
    parent: Option<ActivityId>,
    name: String,
    cancellable: bool,
    attributes: serde_json::Map<String, serde_json::Value>,
    context: ExperimentRunHandle,
    meter: Option<ActivityMeter>,
}

impl ActivityBuilder {
    /// Create a root activity builder.
    pub(crate) fn new(
        id_allocator: Arc<AtomicActivityIdAllocator>,
        control: ExperimentRunControl,
        name: impl Into<String>,
        context: ExperimentRunHandle,
    ) -> Self {
        let cancellation_parent = control.cancel_token();

        Self {
            id_allocator,
            control,
            cancellation_parent,
            parent: None,
            name: name.into(),
            cancellable: false,
            attributes: serde_json::Map::new(),
            context,
            meter: None,
        }
    }

    /// Create a detached builder when its run has already finished.
    ///
    /// Events from a builder on a finished run go nowhere so the API remains infallible.
    pub(crate) fn detached(
        name: impl Into<String>,
        parent: Option<ActivityId>,
        cancel_token: CancelToken,
        context_handle: ExperimentRunHandle,
    ) -> Self {
        Self::new(
            Arc::new(AtomicActivityIdAllocator::new()),
            ExperimentRunControl::default(),
            name,
            context_handle,
        )
        .with_parent(parent, cancel_token)
    }

    /// Declare a numeric meter with an expected total and unit.
    pub fn meter(mut self, total: u64, unit: impl Into<String>) -> Self {
        self.meter = Some(ActivityMeter {
            unit: Some(unit.into()),
            total: Some(total),
        });
        self
    }

    /// Start this activity and return its lifecycle guard.
    pub fn start(self) -> ActivityGuard {
        self.start_inner()
    }

    /// Run a closure inside this activity and finish it according to the result.
    pub fn run<T, E>(self, f: impl FnOnce(Activity) -> Result<T, E>) -> Result<T, E>
    where
        E: std::fmt::Display,
    {
        run_activity(self.start(), f)
    }

    pub(crate) fn with_parent(
        mut self,
        parent: Option<ActivityId>,
        cancellation_parent: CancelToken,
    ) -> Self {
        self.parent = parent;
        self.cancellation_parent = cancellation_parent;
        self
    }

    /// Allow this activity to be cancelled by a remote controller.
    ///
    /// The activity always has a local cancellation token that participates in parent/run
    /// cancellation propagation. Marking it cancellable also exposes that token to remote control.
    pub fn cancellable(mut self) -> Self {
        self.cancellable = true;
        self
    }

    /// Add one attribute.
    pub fn attr(mut self, key: impl Into<String>, value: impl Into<serde_json::Value>) -> Self {
        self.attributes.insert(key.into(), value.into());
        self
    }

    /// Add pre-serialized attributes.
    pub fn attrs(
        mut self,
        attributes: impl IntoIterator<Item = (String, serde_json::Value)>,
    ) -> Self {
        self.attributes.extend(attributes);
        self
    }

    fn start_inner(self) -> ActivityGuard {
        let id = self.id_allocator.next_id();
        let cancel_token = self.cancellation_parent.linked(CancelToken::new());
        let context = self.context.for_activity(id, cancel_token.clone());

        let spec = ActivitySpec {
            id,
            parent: self.parent,
            name: self.name,
            cancellable: self.cancellable,
            meter: self.meter,
            attributes: self.attributes,
        };
        let cancellable = spec.cancellable;

        self.context
            .emit(Event::Activity(ActivityEvent::Started { activity: spec }));

        if cancellable {
            self.control
                .register_activity_cancellation(id, cancel_token.clone());
        }

        let activity = Activity {
            handle: context,
            state: Arc::new(ActivityState {
                id,
                current: AtomicU64::new(0),
                finished: AtomicBool::new(false),
                control: self.control,
                cancellable,
            }),
        };

        ActivityGuard { activity }
    }
}

fn run_activity<T, E>(
    guard: ActivityGuard,
    f: impl FnOnce(Activity) -> Result<T, E>,
) -> Result<T, E>
where
    E: std::fmt::Display,
{
    let activity = guard.share();
    let ambient = activity.clone();
    match ambient.in_scope(|| f(activity)) {
        Ok(value) => {
            guard.finish();
            Ok(value)
        }
        Err(error) => {
            let message = error.to_string();
            guard.fail(message);
            Err(error)
        }
    }
}

/// Shared state of one running activity, held by every reference to it.
struct ActivityState {
    id: ActivityId,
    current: AtomicU64,
    finished: AtomicBool,
    control: ExperimentRunControl,
    cancellable: bool,
}

/// A cloneable reference to a running activity for telemetry and child work.
///
/// The reference does not own the experiment or activity lifecycle. It becomes inactive when the
/// originating run finishes, while retaining the activity's cancellation token.
#[derive(Clone)]
pub struct Activity {
    pub(crate) handle: ExperimentRunHandle,
    state: Arc<ActivityState>,
}

/// Converting this borrow clones the activity's scope-carrying handle, so telemetry emitted
/// through the resulting handle is attributed to that scope.
///
/// Custom context types integrate by implementing `From<&TheirType>` for [`ExperimentRunHandle`].
impl From<&Activity> for ExperimentRunHandle {
    fn from(value: &Activity) -> Self {
        value.handle.clone()
    }
}

impl Activity {
    /// Return the activity identifier.
    pub fn id(&self) -> ActivityId {
        self.state.id
    }

    /// Return a reference whose logs inherit an additional attribute.
    #[must_use]
    pub fn with_attr(&self, key: impl Into<String>, value: impl Into<serde_json::Value>) -> Self {
        Self {
            handle: self.handle.with_attr(key, value),
            state: self.state.clone(),
        }
    }

    /// Return a reference whose logs inherit additional attributes.
    #[must_use]
    pub fn with_attrs(&self, attrs: impl IntoIterator<Item = (String, serde_json::Value)>) -> Self {
        Self {
            handle: self.handle.with_attrs(attrs),
            state: self.state.clone(),
        }
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

    /// Return the activity cancellation token.
    pub fn cancel_token(&self) -> CancelToken {
        self.handle.cancel_token()
    }

    /// Return whether cancellation has been requested for this activity.
    ///
    /// This is a cooperative signal inherited from the run or parent activity. It does not force
    /// the activity's terminal status.
    pub fn is_cancel_requested(&self) -> bool {
        self.handle.cancel_token().is_cancelled()
    }

    /// Return whether the underlying run is still accepting telemetry.
    ///
    /// Telemetry emitted against an inactive run is discarded rather than reported as an error, so
    /// use this when you need to branch on liveness.
    pub fn is_active(&self) -> bool {
        self.handle.is_active()
    }

    /// Increase the current progress value by `delta` and emit the absolute result.
    ///
    /// Activities without a declared meter accept updates as open-ended counts.
    pub fn inc(&self, delta: u64) {
        let mut current = self.state.current.load(Ordering::Relaxed);
        let current = loop {
            let updated = current.saturating_add(delta);
            match self.state.current.compare_exchange_weak(
                current,
                updated,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break updated,
                Err(actual) => current = actual,
            }
        };

        self.report_update(current);
    }

    /// Set the current progress value and emit the absolute result.
    ///
    /// Activities without a declared meter accept updates as open-ended counts.
    pub fn set(&self, current: u64) {
        self.state.current.store(current, Ordering::Relaxed);
        self.report_update(current);
    }

    /// Emit a human-readable message for this activity.
    pub fn message(&self, message: impl Into<String>) {
        self.handle.emit(Event::Activity(ActivityEvent::Message {
            id: self.id(),
            message: message.into(),
        }));
    }

    /// Enter this activity as the ambient telemetry context on the current thread.
    pub fn enter(&self) -> CurrentExperimentGuard {
        self.handle.enter()
    }

    /// Run a closure with this activity installed as the ambient telemetry context.
    pub fn in_scope<T>(&self, f: impl FnOnce() -> T) -> T {
        self.handle.in_scope(f)
    }

    fn report_update(&self, current: u64) {
        self.handle.emit(Event::Activity(ActivityEvent::Updated {
            id: self.state.id,
            current,
        }));
    }

    /// Emit the terminal event exactly once and release any cancellation registration.
    fn complete(&self, status: ActivityStatus, message: Option<String>) {
        if self.state.finished.swap(true, Ordering::AcqRel) {
            return;
        }

        self.handle.emit(Event::Activity(ActivityEvent::Finished {
            id: self.state.id,
            status,
            message,
        }));
        if self.state.cancellable {
            self.state
                .control
                .unregister_activity_cancellation(self.state.id);
        }
    }
}

impl std::fmt::Debug for Activity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Activity")
            .field("experiment_id", self.handle.id())
            .field("activity_id", &self.id())
            .finish_non_exhaustive()
    }
}

/// Owner of an activity's ending; everything else lives on the [`Activity`] it derefs to.
///
/// The shared finished flag makes the terminal event single-shot, so a consuming finisher
/// followed by drop reports exactly once.
pub struct ActivityGuard {
    pub(crate) activity: Activity,
}

/// Converting this borrow clones the activity's scope-carrying handle, so telemetry emitted
/// through the resulting handle is attributed to that scope.
///
/// Custom context types integrate by implementing `From<&TheirType>` for [`ExperimentRunHandle`].
impl From<&ActivityGuard> for ExperimentRunHandle {
    fn from(value: &ActivityGuard) -> Self {
        Self::from(&value.activity)
    }
}

impl ActivityGuard {
    /// Clone the activity reference without transferring lifecycle ownership.
    pub fn share(&self) -> Activity {
        self.activity.clone()
    }

    /// Mark the activity as successful.
    pub fn finish(self) {
        self.activity.complete(ActivityStatus::Success, None);
    }

    /// Mark the activity as successful with a message.
    pub fn finish_with_message(self, message: impl Into<String>) {
        self.activity
            .complete(ActivityStatus::Success, Some(message.into()));
    }

    /// Mark the activity as abandoned.
    pub fn abandon(self) {
        self.activity.complete(ActivityStatus::Abandoned, None);
    }

    /// Mark the activity as abandoned with a message.
    pub fn abandon_with_message(self, message: impl Into<String>) {
        self.activity
            .complete(ActivityStatus::Abandoned, Some(message.into()));
    }

    /// Mark the activity as failed with a message.
    ///
    /// Distinct from [`ActivityGuard::abandon`], which reports work that stopped without an error.
    pub fn fail(self, message: impl Into<String>) {
        self.activity
            .complete(ActivityStatus::Failed, Some(message.into()));
    }
}

impl Drop for ActivityGuard {
    fn drop(&mut self) {
        self.activity.complete(ActivityStatus::Abandoned, None);
    }
}

impl std::ops::Deref for ActivityGuard {
    type Target = Activity;

    fn deref(&self) -> &Self::Target {
        &self.activity
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use crate::test_support::{MockSession, create_run, create_run_with_control};

    use super::*;

    fn setup_run() -> (Arc<MockSession>, crate::ExperimentRun) {
        let session = Arc::new(MockSession::default());
        let run = create_run(session.clone());
        (session, run)
    }

    #[test]
    fn start_reports_configured_activity() {
        let (session, run) = setup_run();

        let _guard = run
            .activity("load")
            .meter(12, "items")
            .attr("split", "train")
            .start();

        let events = session.activity_events();
        let ActivityEvent::Started { activity } = &events[0] else {
            panic!("unexpected event: {:?}", events[0]);
        };
        assert_eq!(activity.name, "load");
        let meter = activity.meter.as_ref().expect("expected activity meter");
        assert_eq!(meter.total, Some(12));
        assert_eq!(meter.unit.as_deref(), Some("items"));
        assert_eq!(activity.attributes.get("split"), Some(&json!("train")));
    }

    #[test]
    fn inc_reports_updated_progress() {
        let (session, run) = setup_run();
        let guard = run.activity("items").meter(8, "items").start();

        guard.inc(3);

        let events = session.activity_events();
        let ActivityEvent::Updated { current, .. } = &events[1] else {
            panic!("unexpected event: {:?}", events[1]);
        };
        assert_eq!(*current, 3);
    }

    #[test]
    fn cloned_activity_reports_accumulated_progress_from_another_thread() {
        let (session, run) = setup_run();
        let guard = run.activity("items").meter(8, "items").start();
        let activity = guard.share();

        activity.inc(2);
        let worker_activity = activity.clone();
        std::thread::spawn(move || worker_activity.inc(3))
            .join()
            .unwrap();

        let updates = session
            .activity_events()
            .into_iter()
            .filter_map(|event| match event {
                ActivityEvent::Updated { current, .. } => Some(current),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(updates, vec![2, 5]);
    }

    #[test]
    fn meterless_activity_reports_updates_as_open_ended_counts() {
        let (session, run) = setup_run();
        let guard = run.activity("items").start();
        let activity = guard.share();

        activity.inc(4);

        let events = session.activity_events();
        assert!(matches!(
            &events[0],
            ActivityEvent::Started { activity } if activity.meter.is_none()
        ));
        assert!(matches!(
            &events[1],
            ActivityEvent::Updated { current: 4, .. }
        ));
    }

    #[test]
    fn message_reports_activity_message() {
        let (session, run) = setup_run();
        let guard = run.activity("items").start();

        guard.message("halfway");

        let events = session.activity_events();
        assert!(matches!(
            &events[1],
            ActivityEvent::Message { id, message }
                if *id == guard.id() && message == "halfway"
        ));
    }

    #[test]
    fn finish_reports_one_success_completion() {
        let (session, run) = setup_run();

        run.activity("node").start().finish();

        let events = session.activity_events();
        let finished: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                ActivityEvent::Finished { status, .. } => Some(status),
                _ => None,
            })
            .collect();
        assert!(matches!(finished.as_slice(), [ActivityStatus::Success]));
    }

    #[test]
    fn fail_reports_failed_completion_with_its_message() {
        let (session, run) = setup_run();

        run.activity("node").start().fail("kernel panicked");

        let events = session.activity_events();
        assert!(matches!(
            events.last(),
            Some(ActivityEvent::Finished {
                status: ActivityStatus::Failed,
                message: Some(message),
                ..
            }) if message == "kernel panicked"
        ));
    }

    #[test]
    fn abandon_reports_abandoned_completion() {
        let (session, run) = setup_run();

        run.activity("node").start().abandon();

        let events = session.activity_events();
        assert!(matches!(
            events.last(),
            Some(ActivityEvent::Finished {
                status: ActivityStatus::Abandoned,
                ..
            })
        ));
    }

    #[test]
    fn drop_reports_abandoned_completion() {
        let (session, run) = setup_run();

        drop(run.activity("node").start());

        let events = session.activity_events();
        assert!(matches!(
            events.last(),
            Some(ActivityEvent::Finished {
                status: ActivityStatus::Abandoned,
                ..
            })
        ));
    }

    #[test]
    fn cancellable_activity_reports_control_metadata() {
        let (session, run) = setup_run();

        let _guard = run.activity("node").cancellable().start();

        let events = session.activity_events();
        let ActivityEvent::Started { activity } = &events[0] else {
            panic!("unexpected event: {:?}", events[0]);
        };
        assert!(activity.cancellable);
    }

    #[test]
    fn cancellable_activity_registers_with_control() {
        let session = Arc::new(MockSession::default());
        let control = ExperimentRunControl::default();
        let run = create_run_with_control(session, control.clone());
        let guard = run.activity("node").cancellable().start();

        assert!(control.cancel_activity(guard.id()));
        assert!(guard.is_cancel_requested());
    }

    #[test]
    fn child_activity_cancel_request_is_linked_to_parent_activity_token() {
        let (_session, run) = setup_run();
        let parent = run.activity("parent").start();
        let child = parent.activity("child").start();

        parent.cancel_token().cancel();

        assert!(parent.is_cancel_requested());
        assert!(child.is_cancel_requested());
    }

    #[test]
    fn drop_reports_abandoned_even_when_cancel_was_requested() {
        let (session, run) = setup_run();
        let guard = run.activity("node").start();

        guard.cancel_token().cancel();
        drop(guard);

        let events = session.activity_events();
        assert!(matches!(
            events.last(),
            Some(ActivityEvent::Finished {
                status: ActivityStatus::Abandoned,
                ..
            })
        ));
    }
}
