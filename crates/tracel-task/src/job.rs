use alloc::boxed::Box;
use core::fmt;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use crate::spawn::{DynFuture, MaybeSend, Spawn};
use crate::task::Task;

/// Work that has not started.
///
/// Await it on any executor, [`block`](Job::block) on it at a native sync edge, or hand it to a
/// [`Spawn`] with [`spawn_on`](Job::spawn_on) and get a running [`Task`] back. Dropping a job
/// before it completes cancels it, as with any future.
///
/// A job composes handles already in flight with in-memory work and nothing else — no timer, no
/// socket, no file descriptor — so polling it needs no reactor, which is what lets any executor
/// drive it and a thread block on it.
#[must_use = "a job does nothing until awaited, blocked on, or spawned"]
pub struct Job<T, E> {
    future: DynFuture<'static, Result<T, E>>,
}

impl<T, E> Job<T, E> {
    /// Wraps `future` as work to be driven later.
    pub fn new<F>(future: F) -> Self
    where
        F: Future<Output = Result<T, E>> + MaybeSend + 'static,
    {
        Self {
            future: Box::pin(future),
        }
    }

    /// Starts the work on `spawn` and returns the handle to its result.
    pub fn spawn_on<S>(self, spawn: &S) -> Task<T, E>
    where
        S: Spawn + ?Sized,
        T: Send + 'static,
        E: Send + 'static,
    {
        Task::spawn(spawn, self.future)
    }

    /// Drives the work to completion on the current thread.
    ///
    /// Needs no runtime, since the handles inside wake the thread through their channels. Would
    /// stall an executor's thread if called from inside one; await there instead. Absent where no
    /// other thread can make progress in the meantime.
    #[cfg(all(feature = "std", not(target_arch = "wasm32")))]
    pub fn block(self) -> Result<T, E> {
        futures::executor::block_on(self)
    }
}

impl<T, E> Future for Job<T, E> {
    type Output = Result<T, E>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.future.as_mut().poll(cx)
    }
}

impl<T, E> fmt::Debug for Job<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Job").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;
    use crate::spawn::ThreadSpawn;
    use crate::task::Aborted;

    #[test]
    fn given_job_when_not_driven_then_nothing_runs() {
        let ran = Arc::new(AtomicBool::new(false));
        let flag = ran.clone();

        let job = Job::<(), Aborted>::new(async move {
            flag.store(true, Ordering::SeqCst);
            Ok(())
        });
        drop(job);

        assert!(!ran.load(Ordering::SeqCst));
    }

    #[test]
    fn given_job_when_blocked_on_then_it_runs_on_the_calling_thread() {
        let job = Job::<_, Aborted>::new(async { Ok(std::thread::current().id()) });

        assert_eq!(job.block(), Ok(std::thread::current().id()));
    }

    #[test]
    fn given_job_when_spawned_then_the_task_yields_its_result() {
        let job = Job::<_, Aborted>::new(async { Ok(7) });

        let task = job.spawn_on(&ThreadSpawn);

        assert_eq!(task.block(), Ok(7));
    }

    #[test]
    fn given_job_over_a_running_task_when_dropped_then_that_task_is_aborted() {
        let done = Arc::new(AtomicBool::new(false));
        let flag = done.clone();
        let (release, gate) = futures::channel::oneshot::channel::<()>();
        let inner = Task::spawn(&ThreadSpawn, async move {
            gate.await.ok();
            flag.store(true, Ordering::SeqCst);
            Ok::<_, Aborted>(())
        });

        let job = Job::new(async move { inner.await });
        drop(job);
        let _ = release.send(());

        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(!done.load(Ordering::SeqCst), "the inner task ran on");
    }
}
