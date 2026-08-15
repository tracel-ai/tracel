//! Activity tracking primitives for experiment runs.
//!
//! Progress is modeled as a tree of named activities. Starting an activity emits a
//! [`ActivityEvent::Started`] event, numeric updates emit [`ActivityEvent::Updated`],
//! and explicit or drop-based completion emits [`ActivityEvent::Finished`].

use std::{
    num::NonZeroU64,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use crate::cancellation::CancelToken;
use crate::control::ExperimentRunControl;
use crate::{Activity, ExperimentRunHandle};

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

/// Sink for activity events.
pub trait ActivityEventReporter: Send + Sync {
    /// Report one activity event.
    fn report(&self, event: ActivityEvent);
}

impl<F> ActivityEventReporter for F
where
    F: Fn(ActivityEvent) + Send + Sync,
{
    fn report(&self, event: ActivityEvent) {
        self(event);
    }
}

/// Allocates unique activity identifiers.
pub trait ActivityIdAllocator: Send + Sync {
    /// Return the next identifier.
    fn next_id(&self) -> ActivityId;
}

/// Lock-free activity identifier allocator.
#[derive(Debug)]
pub struct AtomicActivityIdAllocator {
    next: AtomicU64,
}

impl AtomicActivityIdAllocator {
    /// Create an allocator that starts at identifier `1`.
    pub fn new() -> Self {
        Self {
            next: AtomicU64::new(1),
        }
    }
}

impl Default for AtomicActivityIdAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl ActivityIdAllocator for AtomicActivityIdAllocator {
    fn next_id(&self) -> ActivityId {
        let id = self.next.fetch_add(1, Ordering::Relaxed);

        // Starts at 1, so this should only fail after overflow or wraparound.
        let id = NonZeroU64::new(id).expect("activity id allocator overflowed or produced zero");

        ActivityId(id)
    }
}

/// Builder used to configure and start an activity.
pub struct ActivityBuilder {
    reporter: Arc<dyn ActivityEventReporter>,
    id_allocator: Arc<dyn ActivityIdAllocator>,
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
        reporter: Arc<dyn ActivityEventReporter>,
        id_allocator: Arc<dyn ActivityIdAllocator>,
        control: ExperimentRunControl,
        name: impl Into<String>,
        context: ExperimentRunHandle,
    ) -> Self {
        let cancellation_parent = control.cancel_token();

        Self {
            reporter,
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
            Arc::new(|_| {}),
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

    /// Add one serializable attribute.
    pub fn attr<T>(mut self, key: impl Into<String>, value: T) -> Result<Self, serde_json::Error>
    where
        T: serde::Serialize,
    {
        self.insert_attr(key, value)?;
        Ok(self)
    }

    /// Add pre-serialized attributes.
    pub fn attrs<T>(mut self, attributes: T) -> Result<Self, serde_json::Error>
    where
        T: IntoIterator<Item = (String, serde_json::Value)>,
    {
        for (key, value) in attributes {
            self.attributes.insert(key, value);
        }
        Ok(self)
    }

    fn insert_attr<T>(&mut self, key: impl Into<String>, value: T) -> Result<(), serde_json::Error>
    where
        T: serde::Serialize,
    {
        self.attributes
            .insert(key.into(), serde_json::to_value(value)?);
        Ok(())
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

        self.reporter
            .report(ActivityEvent::Started { activity: spec });

        if cancellable {
            self.control
                .register_activity_cancellation(id, cancel_token.clone());
        }

        let activity = Activity::new(context, id, self.reporter.clone());
        let active = ActiveActivity::new(self.reporter, self.control, id, cancellable);

        ActivityGuard { activity, active }
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

/// Lifecycle state for a running activity.
struct ActiveActivity {
    reporter: Arc<dyn ActivityEventReporter>,
    control: ExperimentRunControl,
    id: ActivityId,
    cancellable: bool,
    finished: bool,
}

impl ActiveActivity {
    /// Create lifecycle state for an already-started activity.
    fn new(
        reporter: Arc<dyn ActivityEventReporter>,
        control: ExperimentRunControl,
        id: ActivityId,
        cancellable: bool,
    ) -> Self {
        Self {
            reporter,
            control,
            id,
            cancellable,
            finished: false,
        }
    }

    fn finish_inner(&mut self, status: ActivityStatus, message: Option<String>) {
        if self.finished {
            return;
        }

        self.finished = true;
        self.reporter.report(ActivityEvent::Finished {
            id: self.id,
            status,
            message,
        });
        if self.cancellable {
            self.control.unregister_activity_cancellation(self.id);
        }
    }
}

impl Drop for ActiveActivity {
    fn drop(&mut self) {
        self.finish_inner(ActivityStatus::Abandoned, None);
    }
}

/// Lifecycle guard for a running activity.
pub struct ActivityGuard {
    pub(crate) activity: Activity,
    active: ActiveActivity,
}

impl ActivityGuard {
    /// Clone the activity reference without transferring lifecycle ownership.
    pub fn share(&self) -> Activity {
        self.activity.clone()
    }

    /// Mark the activity as successful.
    pub fn finish(mut self) {
        self.active.finish_inner(ActivityStatus::Success, None);
    }

    /// Mark the activity as successful with a message.
    pub fn finish_with_message(mut self, message: impl Into<String>) {
        self.active
            .finish_inner(ActivityStatus::Success, Some(message.into()));
    }

    /// Mark the activity as abandoned.
    pub fn abandon(mut self) {
        self.active.finish_inner(ActivityStatus::Abandoned, None);
    }

    /// Mark the activity as abandoned with a message.
    pub fn abandon_with_message(mut self, message: impl Into<String>) {
        self.active
            .finish_inner(ActivityStatus::Abandoned, Some(message.into()));
    }

    /// Mark the activity as failed with a message.
    ///
    /// Failed activities use the abandoned status until a distinct activity failure status is
    /// available on the wire.
    pub fn fail(mut self, message: impl Into<String>) {
        self.active
            .finish_inner(ActivityStatus::Abandoned, Some(message.into()));
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
    use std::sync::Mutex;

    use serde_json::json;

    use crate::ExperimentContext as _;

    use super::*;

    #[derive(Default)]
    struct MockReporter {
        events: Mutex<Vec<ActivityEvent>>,
    }

    impl MockReporter {
        fn events(&self) -> Vec<ActivityEvent> {
            self.events.lock().unwrap().clone()
        }
    }

    impl ActivityEventReporter for MockReporter {
        fn report(&self, event: ActivityEvent) {
            self.events.lock().unwrap().push(event);
        }
    }

    fn test_context(control: ExperimentRunControl) -> ExperimentRunHandle {
        let context_cancel_token = control.cancel_token();
        ExperimentRunHandle {
            metadata: crate::ExperimentMetadata {
                id: crate::ExperimentId::new("test/experiment/1"),
            },
            inner: std::sync::Weak::new(),
            control,
            activity: None,
            context_cancel_token,
            scope: Arc::new(serde_json::Map::new()),
        }
    }

    fn builder(reporter: Arc<MockReporter>, name: &str) -> ActivityBuilder {
        let control = ExperimentRunControl::default();
        ActivityBuilder::new(
            reporter,
            Arc::new(AtomicActivityIdAllocator::new()),
            control.clone(),
            name.to_string(),
            test_context(control),
        )
    }

    #[test]
    fn start_reports_configured_activity() {
        let reporter = Arc::new(MockReporter::default());

        let _guard = builder(reporter.clone(), "load")
            .meter(12, "items")
            .attr("split", "train")
            .unwrap()
            .start();

        let events = reporter.events();
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
    fn activity_start_reports_no_meter() {
        let reporter = Arc::new(MockReporter::default());

        let _guard = builder(reporter.clone(), "epoch").start();

        let events = reporter.events();
        let ActivityEvent::Started { activity } = &events[0] else {
            panic!("unexpected event: {:?}", events[0]);
        };
        assert_eq!(activity.name, "epoch");
        assert!(activity.meter.is_none());
    }

    #[test]
    fn inc_reports_updated_progress() {
        let reporter = Arc::new(MockReporter::default());
        let guard = builder(reporter.clone(), "items").meter(8, "items").start();

        guard.inc(3);

        let events = reporter.events();
        let ActivityEvent::Updated { current, .. } = &events[1] else {
            panic!("unexpected event: {:?}", events[1]);
        };
        assert_eq!(*current, 3);
    }

    #[test]
    fn cloned_activity_reports_accumulated_progress_from_another_thread() {
        let reporter = Arc::new(MockReporter::default());
        let guard = builder(reporter.clone(), "items").meter(8, "items").start();
        let activity = guard.share();

        activity.inc(2);
        let worker_activity = activity.clone();
        std::thread::spawn(move || worker_activity.inc(3))
            .join()
            .unwrap();

        let updates = reporter
            .events()
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
        let reporter = Arc::new(MockReporter::default());
        let guard = builder(reporter.clone(), "items").start();
        let activity = guard.share();

        activity.inc(4);

        let events = reporter.events();
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
    fn finish_reports_one_success_completion() {
        let reporter = Arc::new(MockReporter::default());

        builder(reporter.clone(), "node").start().finish();

        let events = reporter.events();
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
    fn drop_reports_abandoned_completion() {
        let reporter = Arc::new(MockReporter::default());

        drop(builder(reporter.clone(), "node").start());

        let events = reporter.events();
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
        let reporter = Arc::new(MockReporter::default());

        let _guard = builder(reporter.clone(), "node").cancellable().start();

        let events = reporter.events();
        let ActivityEvent::Started { activity } = &events[0] else {
            panic!("unexpected event: {:?}", events[0]);
        };
        assert!(activity.cancellable);
    }

    #[test]
    fn cancellable_activity_registers_with_control() {
        let reporter = Arc::new(MockReporter::default());
        let control = ExperimentRunControl::default();
        let guard = ActivityBuilder::new(
            reporter,
            Arc::new(AtomicActivityIdAllocator::new()),
            control.clone(),
            "node",
            test_context(control.clone()),
        )
        .cancellable()
        .start();

        assert!(control.cancel_activity(guard.id()));
        assert!(guard.is_cancel_requested());
    }

    #[test]
    fn child_activity_cancel_request_is_linked_to_parent_activity_token() {
        let reporter = Arc::new(MockReporter::default());
        let parent = builder(reporter, "parent").start();
        let child = parent.activity("child").start();

        parent.cancel_token().cancel();

        assert!(parent.is_cancel_requested());
        assert!(child.is_cancel_requested());
    }

    #[test]
    fn drop_reports_abandoned_even_when_cancel_was_requested() {
        let reporter = Arc::new(MockReporter::default());
        let guard = builder(reporter.clone(), "node").start();

        guard.cancel_token().cancel();
        drop(guard);

        let events = reporter.events();
        assert!(matches!(
            events.last(),
            Some(ActivityEvent::Finished {
                status: ActivityStatus::Abandoned,
                ..
            })
        ));
    }
}
