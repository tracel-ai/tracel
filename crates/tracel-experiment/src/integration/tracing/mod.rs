//! Tracing integration for routing `tracing` events into experiments.
//!
//! Install [`tracing_log_layer`] or call [`try_init_tracing_subscriber`] to enable forwarding.
//! Once installed, you can choose between two routing styles:
//! - ambient routing with [`crate::ExperimentGlobalExt::enter`],
//!   [`crate::ExperimentGlobalExt::in_scope`], or
//!   [`crate::ExperimentInstrument::in_experiment`]
//! - explicit span routing with [`ExperimentTracingExt::tracing_span`]
//!
//! Ambient routing is usually the simplest option when your code already has access to an
//! [`crate::ExperimentRun`] or [`crate::ExperimentRunHandle`]. Span routing is useful when you
//! need to bind logs to an experiment without relying on thread-local ambient state.

use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use crate::{ActivityGuard, ActivityId, ExperimentRun, ExperimentRunHandle};

mod layer;
pub(crate) mod registry;
mod visitor;

pub use layer::ExperimentTracingLogLayer;

/// Create a layer that forwards `tracing` events to the experiment associated with the current
/// tracing scope.
///
/// This is a convenience constructor for [`ExperimentTracingLogLayer`].
pub fn tracing_log_layer() -> ExperimentTracingLogLayer {
    ExperimentTracingLogLayer
}

/// Best-effort initialization of a default tracing subscriber that includes experiment log
/// forwarding.
///
/// Returns `true` when a subscriber was installed and `false` when one was already installed or
/// initialization otherwise failed.
///
/// This is a convenience for binaries that do not already configure their own subscriber.
pub fn try_init_tracing_subscriber() -> bool {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_log_layer())
        .try_init()
        .is_ok()
}

/// Build an explicit tracing span tied to a specific experiment.
fn experiment_span(experiment: impl Into<ExperimentRunHandle>) -> tracing::Span {
    let experiment = experiment.into();
    tracing::info_span!("experiment", experiment_id = %experiment.id())
}

/// Extension trait for creating explicit experiment-bound tracing spans.
///
/// Implemented for both [`ExperimentRun`] and [`ExperimentRunHandle`].
pub trait ExperimentTracingExt {
    /// Create a tracing span bound to this experiment.
    ///
    /// Use this when ambient routing through [`crate::ExperimentGlobalExt`] is not practical or
    /// when you want the routing to be explicit in the span tree.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use tracel_experiment::ExperimentRun;
    /// use tracel_experiment::integration::tracing::{ExperimentTracingExt};
    ///
    /// # fn main() {
    /// let experiment = ExperimentRun::local("/tmp/experiments").unwrap();
    /// let span: tracing::Span = experiment.tracing_span();
    /// let _guard = span.enter();
    /// tracing::info!("this event is routed to the experiment");
    /// # }
    /// ```
    #[must_use = "span must be entered to route events"]
    fn tracing_span(&self) -> tracing::Span;
}

impl ExperimentTracingExt for ExperimentRun {
    fn tracing_span(&self) -> tracing::Span {
        experiment_span(self)
    }
}

impl ExperimentTracingExt for ExperimentRunHandle {
    fn tracing_span(&self) -> tracing::Span {
        experiment_span(self.clone())
    }
}

/// Build a tracing span scoped to a specific activity.
fn activity_span(id: ActivityId) -> tracing::Span {
    tracing::info_span!("activity", activity_id = id.as_u64())
}

/// Extension trait for creating activity-scoped tracing spans.
///
/// Entering the returned span scopes `tracing` events emitted within it to this activity.
pub trait ActivityTracingExt {
    /// Create a tracing span scoped to this activity.
    #[must_use = "span must be entered to scope events"]
    fn tracing_span(&self) -> tracing::Span;
}

impl<State> ActivityTracingExt for ActivityGuard<State> {
    fn tracing_span(&self) -> tracing::Span {
        activity_span(self.id())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tracing_subscriber::layer::SubscriberExt;

    use crate::context::ExperimentGlobalExt;
    use crate::error::ExperimentError;
    use crate::reader::{ExperimentArtifactReader, ExperimentReaderError, LoadedArtifact};
    use crate::session::{BundleFn, Event, ExperimentCompletion, ExperimentSession};
    use crate::{ArtifactKind, CancelToken, ExperimentId, ExperimentRun};

    use super::*;

    #[derive(Default)]
    struct MockSession {
        events: Mutex<Vec<Event>>,
    }

    impl ExperimentSession for MockSession {
        fn record_event(&self, event: Event) -> Result<(), ExperimentError> {
            self.events.lock().unwrap().push(event);
            Ok(())
        }

        fn save_artifact(
            &self,
            _name: &str,
            _kind: ArtifactKind,
            _artifact: Box<BundleFn>,
        ) -> Result<(), ExperimentError> {
            Ok(())
        }

        fn finish(&self, _completion: ExperimentCompletion) -> Result<(), ExperimentError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct NoopExperimentDataReader;

    impl ExperimentArtifactReader for NoopExperimentDataReader {
        fn load_artifact_raw(
            &self,
            _experiment_id: ExperimentId,
            _name: &str,
        ) -> Result<LoadedArtifact, ExperimentReaderError> {
            Err(ExperimentReaderError::new("Artifact not found"))
        }
    }

    fn create_run(id: &str, session: Arc<MockSession>) -> ExperimentRun {
        ExperimentRun::new(
            id,
            session,
            NoopExperimentDataReader,
            CancelToken::default(),
        )
    }

    #[test]
    fn tracing_layer_forwards_events_to_current_experiment() {
        let session = Arc::new(MockSession::default());
        let run = create_run("trace-test-1", session.clone());
        let subscriber = tracing_subscriber::registry().with(tracing_log_layer());

        tracing::subscriber::with_default(subscriber, || {
            run.in_scope(|| {
                tracing::info!(step = 3u64, "epoch completed");
            })
        });

        let events = session.events.lock().unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::Log(record) => {
                assert!(record.message.contains("epoch completed"));
                assert_eq!(
                    record.attributes.get("step").and_then(|v| v.as_u64()),
                    Some(3)
                );
                assert_eq!(record.activity_id, None);
            }
            event => panic!("unexpected event: {event:?}"),
        }
    }

    #[test]
    fn tracing_layer_routes_from_span_experiment_id_without_ambient_scope() {
        let session = Arc::new(MockSession::default());
        let run = create_run("trace-test-span", session.clone());
        let subscriber = tracing_subscriber::registry().with(tracing_log_layer());

        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!("experiment", experiment_id = %run.id());
            let _guard = span.enter();
            tracing::info!(step = 7u64, "span-routed event");
        });

        let events = session.events.lock().unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::Log(record) => {
                assert!(record.message.contains("span-routed event"));
                assert_eq!(
                    record.attributes.get("step").and_then(|v| v.as_u64()),
                    Some(7)
                );
            }
            event => panic!("unexpected event: {event:?}"),
        }
    }

    #[test]
    fn tracing_layer_routes_from_experiment_span_helper_without_ambient_scope() {
        let session = Arc::new(MockSession::default());
        let run = create_run("trace-test-helper-span", session.clone());
        let subscriber = tracing_subscriber::registry().with(tracing_log_layer());

        tracing::subscriber::with_default(subscriber, || {
            let span = experiment_span(&run);
            let _guard = span.enter();
            tracing::info!("helper-span-routed event");
        });

        let events = session.events.lock().unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::Log(record) => {
                assert!(record.message.contains("helper-span-routed event"));
            }
            event => panic!("unexpected event: {event:?}"),
        }
    }

    #[test]
    fn tracing_layer_skips_events_without_experiment_scope() {
        let session = Arc::new(MockSession::default());
        let _run = create_run("trace-test-2", session.clone());
        let subscriber = tracing_subscriber::registry().with(tracing_log_layer());

        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("outside experiment scope");
        });

        let events = session.events.lock().unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn tracing_layer_scopes_events_to_the_current_activity() {
        let session = Arc::new(MockSession::default());
        let run = create_run("trace-test-activity", session.clone());
        let subscriber = tracing_subscriber::registry().with(tracing_log_layer());

        let activity_id = tracing::subscriber::with_default(subscriber, || {
            run.in_scope(|| {
                let guard = run.activity("train").start();
                let id = guard.id();
                guard.tracing_span().in_scope(|| {
                    tracing::info!("scoped to activity");
                });
                id
            })
        });

        let events = session.events.lock().unwrap();
        let log = events
            .iter()
            .find_map(|event| match event {
                Event::Log(record) => Some(record),
                _ => None,
            })
            .expect("a log event should have been recorded");
        assert_eq!(log.message, "scoped to activity");
        assert_eq!(log.activity_id, Some(activity_id));
    }

    #[test]
    fn tracing_layer_inherits_span_field_attributes() {
        let session = Arc::new(MockSession::default());
        let run = create_run("trace-test-scope", session.clone());
        let subscriber = tracing_subscriber::registry().with(tracing_log_layer());

        tracing::subscriber::with_default(subscriber, || {
            run.in_scope(|| {
                let span = tracing::info_span!("phase", stage = "train", shard = 2u64);
                span.in_scope(|| {
                    // The event's own field overrides the inherited `shard` from the span.
                    tracing::info!(shard = 5u64, "step done");
                });
            })
        });

        let events = session.events.lock().unwrap();
        let log = events
            .iter()
            .find_map(|event| match event {
                Event::Log(record) => Some(record),
                _ => None,
            })
            .expect("a log event should have been recorded");
        assert_eq!(log.message, "step done");
        assert_eq!(
            log.attributes.get("stage").and_then(|v| v.as_str()),
            Some("train")
        );
        assert_eq!(
            log.attributes.get("shard").and_then(|v| v.as_u64()),
            Some(5)
        );
    }
}
