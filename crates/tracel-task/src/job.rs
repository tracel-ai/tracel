use alloc::boxed::Box;
use core::fmt;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};

use crate::bounds::{DynFuture, MaybeSend};

/// Work the caller drives.
///
/// Await it on any executor, [`block`](Job::block) on it at a native sync edge, or
/// [`try_poll`](Job::try_poll) it once per tick from a loop that must not suspend. It is a plain
/// future: to run it in the background, spawn it on whatever the caller already has. Dropping
/// it before it completes cancels it.
///
/// A job holds only futures any executor can poll: in-memory work, channel ends, and IO that a
/// backend has attached to the runtime driving it. That is what lets one type serve a thread, a
/// browser, and an embedded executor alike.
#[must_use = "a job does nothing until driven"]
pub struct Job<T, E> {
    state: State<T, E>,
}

enum State<T, E> {
    Ready(Option<Result<T, E>>),
    Pending(DynFuture<'static, Result<T, E>>),
}

impl<T, E> Job<T, E> {
    /// Wraps `future` as work to be driven.
    pub fn new<F>(future: F) -> Self
    where
        F: Future<Output = Result<T, E>> + MaybeSend + 'static,
    {
        Self {
            state: State::Pending(Box::pin(future)),
        }
    }

    /// Creates a job that already holds `value`. Allocates nothing.
    pub fn ready(value: T) -> Self {
        Self::from_result(Ok(value))
    }

    /// Creates a job that already failed with `error`.
    pub fn failed(error: E) -> Self {
        Self::from_result(Err(error))
    }

    /// Creates a job that already holds `result`.
    pub fn from_result(result: Result<T, E>) -> Self {
        Self {
            state: State::Ready(Some(result)),
        }
    }

    /// Advances the work by one poll and takes the result if that completed it.
    ///
    /// For a loop that must not suspend: call it once per tick until it returns `Some`. Readiness
    /// is state-based in every future a job may hold, so progress made between ticks is seen on
    /// the next call even though no waker is registered.
    pub fn try_poll(&mut self) -> Option<Result<T, E>> {
        match &mut self.state {
            State::Ready(slot) => slot.take(),
            State::Pending(future) => {
                let mut cx = Context::from_waker(Waker::noop());
                match future.as_mut().poll(&mut cx) {
                    Poll::Ready(result) => {
                        self.state = State::Ready(None);
                        Some(result)
                    }
                    Poll::Pending => None,
                }
            }
        }
    }

    /// Drives the work to completion on the current thread.
    ///
    /// Parks the thread between polls, so it needs no runtime — and would stall an executor's
    /// thread if called from inside one; await there instead. Absent where no other thread can
    /// make progress in the meantime.
    #[cfg(all(feature = "std", not(target_arch = "wasm32")))]
    pub fn block(self) -> Result<T, E> {
        futures::executor::block_on(self)
    }
}

impl<T, E> Unpin for Job<T, E> {}

impl<T, E> Future for Job<T, E> {
    type Output = Result<T, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        match &mut this.state {
            State::Ready(slot) => Poll::Ready(slot.take().expect("Job polled after completion")),
            State::Pending(future) => {
                let result = core::task::ready!(future.as_mut().poll(cx));
                this.state = State::Ready(None);
                Poll::Ready(result)
            }
        }
    }
}

impl<T, E> fmt::Debug for Job<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = match &self.state {
            State::Ready(Some(_)) => "ready",
            State::Ready(None) => "taken",
            State::Pending(_) => "pending",
        };
        f.debug_struct("Job").field("state", &state).finish()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use futures::channel::oneshot;

    use super::*;

    #[test]
    fn given_ready_job_when_polled_then_value_is_available_immediately() {
        let mut job = Job::<_, ()>::ready(7);

        assert_eq!(job.try_poll(), Some(Ok(7)));
        assert_eq!(job.try_poll(), None);
    }

    #[test]
    fn given_job_when_not_driven_then_nothing_runs() {
        let ran = Arc::new(AtomicBool::new(false));
        let flag = ran.clone();

        let job = Job::<(), ()>::new(async move {
            flag.store(true, Ordering::SeqCst);
            Ok(())
        });
        drop(job);

        assert!(!ran.load(Ordering::SeqCst));
    }

    #[test]
    fn given_job_when_blocked_on_then_it_runs_on_the_calling_thread() {
        let job = Job::<_, ()>::new(async { Ok(std::thread::current().id()) });

        assert_eq!(job.block(), Ok(std::thread::current().id()));
    }

    #[test]
    fn given_pending_job_when_polled_per_tick_then_it_completes_once_its_input_arrives() {
        let (release, gate) = oneshot::channel::<()>();
        let mut job = Job::<_, ()>::new(async move {
            gate.await.ok();
            Ok(7)
        });

        assert_eq!(job.try_poll(), None);
        release.send(()).unwrap();

        assert_eq!(job.try_poll(), Some(Ok(7)));
    }

    #[test]
    fn given_job_when_handed_to_a_thread_then_the_caller_backgrounds_it_with_their_own_tools() {
        let job = Job::<_, ()>::new(async { Ok(7) });

        let worker = std::thread::spawn(move || job.block());

        assert_eq!(worker.join().unwrap(), Ok(7));
    }

    #[test]
    fn given_job_when_awaited_from_another_future_then_it_resolves() {
        let job = Job::<_, ()>::new(async { Ok(7) });

        let result = futures::executor::block_on(async { job.await.map(|value| value + 1) });

        assert_eq!(result, Ok(8));
    }
}
