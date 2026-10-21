//! Ambient experiment context propagation.
//!
//! This module provides two related pieces:
//! - [`ExperimentGlobalExt`] for entering a thread-local ambient experiment scope.
//! - [`ExperimentInstrument`] for re-entering that scope whenever a future is polled.

use std::cell::RefCell;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};

use crate::{ExperimentRun, ExperimentRunHandle};

thread_local! {
    static CURRENT_EXPERIMENTS: RefCell<Vec<AmbientExperiment>> = const { RefCell::new(Vec::new()) };
}

static NEXT_AMBIENT_TOKEN: AtomicU64 = AtomicU64::new(1);

struct AmbientExperiment {
    token: u64,
    handle: ExperimentRunHandle,
}

/// Guard returned when entering an ambient experiment scope.
///
/// Ambient scopes are thread-local and stack-based. When the guard is dropped, the previous
/// ambient experiment on that thread is restored.
///
/// This is returned by [`ExperimentRunHandle::enter`] and [`ExperimentGlobalExt::enter`].
#[derive(Debug)]
#[must_use = "ambient experiment guards must be kept alive to maintain the experiment scope"]
pub struct CurrentExperimentGuard {
    token: u64,
    not_send: PhantomData<Rc<()>>,
}

/// Future wrapper that re-enters the ambient experiment scope on every poll.
///
/// Instances are created through [`ExperimentInstrument`].
pub struct WithCurrentExperiment<F> {
    future: F,
    handle: ExperimentRunHandle,
}

/// Extension trait for propagating an experiment context with a future.
///
/// Use this when a future may be polled on a different thread or after the current ambient scope
/// has ended.
///
/// # Example
///
/// In code that runs in an experiment, given its run, such as code
/// [`Experiments::run`](crate::Experiments::run) runs:
///
/// ```no_run
/// use tracel_experiment::{ExperimentInstrument, ExperimentRun};
///
/// fn train(run: &ExperimentRun) {
///     let _future = async move {
///         tracing::info!("this future will poll inside the experiment context");
///     }
///     .in_experiment(run);
/// }
/// ```
pub trait ExperimentInstrument: Future + Sized {
    /// Instrument this future to automatically enter the given experiment context.
    fn in_experiment<H>(self, experiment: H) -> WithCurrentExperiment<Self>
    where
        H: Into<ExperimentRunHandle>,
    {
        WithCurrentExperiment::new(self, experiment.into())
    }

    /// Instrument this future to automatically enter the current ambient experiment context, if one exists.
    ///
    /// # Panics
    ///
    /// Panics if there is no ambient experiment to propagate.
    fn in_current_experiment(self) -> WithCurrentExperiment<Self> {
        let handle = current_experiment().expect("no ambient experiment to propagate");
        WithCurrentExperiment::new(self, handle)
    }
}

impl<F: Future> ExperimentInstrument for F {}

impl<F> WithCurrentExperiment<F> {
    fn new(future: F, handle: ExperimentRunHandle) -> Self {
        Self { future, handle }
    }
}

/// Extension trait providing methods for entering and propagating an ambient experiment context.
///
/// Ambient context is thread-local and stack-based. Nested scopes temporarily override the current
/// experiment and restore the previous one when their guard is dropped.
///
/// This trait is implemented for [`ExperimentRun`]. [`ExperimentRunHandle`] provides matching
/// [`ExperimentRunHandle::enter`] and [`ExperimentRunHandle::in_scope`] methods when you only have
/// a cloned handle.
///
/// # Example
///
/// In code that runs in an experiment, given its run, such as code
/// [`Experiments::run`](crate::Experiments::run) runs:
///
/// ```no_run
/// use tracel_experiment::{ExperimentGlobalExt, ExperimentRun};
///
/// fn train(run: &ExperimentRun) {
///     run.in_scope(|| {
///         let current = ExperimentRun::current().unwrap();
///         assert_eq!(current.id(), run.id());
///     });
/// }
/// ```
pub trait ExperimentGlobalExt {
    /// Enter an ambient scope for this run until the returned guard is dropped.
    fn enter(&self) -> CurrentExperimentGuard;

    /// Run a closure with this run installed as the ambient experiment.
    fn in_scope<T>(&self, f: impl FnOnce() -> T) -> T;

    /// Return the current ambient experiment handle for this thread, if any.
    fn current() -> Option<ExperimentRunHandle>;
}

impl ExperimentRunHandle {
    /// See [`ExperimentGlobalExt::enter`].
    pub fn enter(&self) -> CurrentExperimentGuard {
        enter_experiment_handle(self.clone())
    }

    /// See [`ExperimentGlobalExt::in_scope`].
    pub fn in_scope<T>(&self, f: impl FnOnce() -> T) -> T {
        with_experiment_handle(self.clone(), f)
    }
}

impl ExperimentGlobalExt for ExperimentRun {
    fn enter(&self) -> CurrentExperimentGuard {
        enter_experiment_handle(self.handle.clone())
    }

    fn in_scope<T>(&self, f: impl FnOnce() -> T) -> T {
        with_experiment_handle(self.handle.clone(), f)
    }

    fn current() -> Option<ExperimentRunHandle> {
        current_experiment()
    }
}

impl<F: Future> Future for WithCurrentExperiment<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: We never move `future` after `self` has been pinned. We only project a mutable
        // reference to it for polling.
        let this = unsafe { self.get_unchecked_mut() };
        let _guard = enter_experiment_handle(this.handle.clone());
        // SAFETY: `future` is pinned together with `self` and is never moved after pinning.
        unsafe { Pin::new_unchecked(&mut this.future) }.poll(cx)
    }
}

/// Return the current ambient experiment handle, if one is in scope on this thread.
fn current_experiment() -> Option<ExperimentRunHandle> {
    CURRENT_EXPERIMENTS.with(|experiments| {
        experiments
            .borrow()
            .last()
            .map(|experiment| experiment.handle.clone())
    })
}

fn enter_experiment_handle(handle: ExperimentRunHandle) -> CurrentExperimentGuard {
    let token = NEXT_AMBIENT_TOKEN.fetch_add(1, Ordering::Relaxed);
    CURRENT_EXPERIMENTS.with(|experiments| {
        experiments
            .borrow_mut()
            .push(AmbientExperiment { token, handle });
    });

    CurrentExperimentGuard {
        token,
        not_send: PhantomData,
    }
}

fn with_experiment_handle<T>(handle: ExperimentRunHandle, f: impl FnOnce() -> T) -> T {
    let _guard = enter_experiment_handle(handle);
    f()
}

impl Drop for CurrentExperimentGuard {
    fn drop(&mut self) {
        CURRENT_EXPERIMENTS.with(|experiments| {
            let mut experiments = experiments.borrow_mut();
            let position = experiments
                .iter()
                .rposition(|experiment| experiment.token == self.token);
            debug_assert!(position.is_some(), "ambient experiment scope is not active");
            if let Some(position) = position {
                experiments.remove(position);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::context::ExperimentGlobalExt as _;
    use crate::test_support::create_run_with_id;
    use crate::{ExperimentId, ExperimentRun};

    use super::ExperimentInstrument;

    fn create_run(id: &str) -> ExperimentRun {
        create_run_with_id(id, Default::default())
    }

    fn current_id() -> Option<ExperimentId> {
        ExperimentRun::current().map(|current| current.id().clone())
    }

    async fn current_id_after_an_await() -> Option<ExperimentId> {
        tokio::task::yield_now().await;
        current_id()
    }

    #[test]
    fn an_inner_scope_overrides_the_outer_one() {
        let outer = create_run("outer");
        let inner = create_run("inner");
        let _outer = outer.enter();

        let _inner = inner.enter();

        assert_eq!(current_id(), Some(inner.id().clone()));
    }

    #[test]
    fn leaving_an_inner_scope_restores_the_outer_one() {
        let outer = create_run("outer");
        let inner = create_run("inner");
        let _outer = outer.enter();

        drop(inner.enter());

        assert_eq!(current_id(), Some(outer.id().clone()));
    }

    #[test]
    fn leaving_an_outer_scope_first_keeps_the_inner_one() {
        let outer = create_run("outer");
        let inner = create_run("inner");
        let outer_scope = outer.enter();
        let _inner_scope = inner.enter();

        drop(outer_scope);

        assert_eq!(current_id(), Some(inner.id().clone()));
    }

    #[test]
    fn a_handle_enters_its_run_on_another_thread() {
        let run = create_run("run");
        let handle = run.handle();

        let current = std::thread::spawn(move || handle.in_scope(current_id))
            .join()
            .unwrap();

        assert_eq!(current, Some(run.id().clone()));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn instrumented_tasks_each_keep_their_own_experiment_across_an_await() {
        let first = create_run("first");
        let second = create_run("second");

        let tasks = [&first, &second]
            .map(|run| tokio::spawn(current_id_after_an_await().in_experiment(run)));
        let mut seen = Vec::new();
        for task in tasks {
            seen.push(task.await.unwrap());
        }

        assert_eq!(seen, [Some(first.id().clone()), Some(second.id().clone())]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_task_spawned_in_the_current_experiment_keeps_it() {
        let run = create_run("run");

        let seen = tokio::spawn(
            async {
                tokio::spawn(current_id_after_an_await().in_current_experiment())
                    .await
                    .unwrap()
            }
            .in_experiment(&run),
        )
        .await
        .unwrap();

        assert_eq!(seen, Some(run.id().clone()));
    }
}
