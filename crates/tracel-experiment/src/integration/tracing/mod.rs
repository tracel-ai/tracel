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

use crate::{Activity, ActivityGuard, ExperimentRun, ExperimentRunHandle};

mod layer;
pub(crate) mod registry;

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
    /// In code that runs in an experiment, given its run, such as code
    /// [`Experiments::run`](crate::Experiments::run) runs:
    ///
    /// ```no_run
    /// use tracel_experiment::ExperimentRun;
    /// use tracel_experiment::integration::tracing::ExperimentTracingExt;
    ///
    /// fn train(experiment: &ExperimentRun) {
    ///     let span: tracing::Span = experiment.tracing_span();
    ///     let _guard = span.enter();
    ///     tracing::info!("this event is routed to the experiment");
    /// }
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
        experiment_span(self)
    }
}

/// Build a tracing span scoped to a specific activity.
fn activity_span(activity: Activity) -> tracing::Span {
    tracing::info_span!(
        "activity",
        experiment_id = %activity.handle.id(),
        activity_id = activity.id().as_u64(),
    )
}

/// Extension trait for creating activity-scoped tracing spans.
///
/// Entering the returned span scopes `tracing` events emitted within it to this activity.
pub trait ActivityTracingExt {
    /// Create a tracing span scoped to this activity.
    #[must_use = "span must be entered to scope events"]
    fn tracing_span(&self) -> tracing::Span;
}

impl ActivityTracingExt for ActivityGuard {
    fn tracing_span(&self) -> tracing::Span {
        activity_span(self.share())
    }
}

impl ActivityTracingExt for Activity {
    fn tracing_span(&self) -> tracing::Span {
        activity_span(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::test_support::{MockSession, create_run_with_id, traced};

    #[test]
    fn an_event_in_an_experiment_tracing_span_is_logged_to_that_experiment() {
        let session = Arc::new(MockSession::default());
        let run = create_run_with_id("tracing-experiment-span", session.clone());

        traced(|| {
            run.tracing_span()
                .in_scope(|| tracing::info!("epoch completed"))
        });

        assert_eq!(session.logs().len(), 1);
    }

    #[test]
    fn an_event_in_an_activity_tracing_span_is_logged_to_that_activity() {
        let session = Arc::new(MockSession::default());
        let run = create_run_with_id("tracing-activity-span", session.clone());
        let activity = run.activity("fold").start();

        traced(|| {
            activity
                .tracing_span()
                .in_scope(|| tracing::info!("training fold"))
        });

        assert_eq!(session.log_activities(), [Some(activity.id())]);
    }
}
