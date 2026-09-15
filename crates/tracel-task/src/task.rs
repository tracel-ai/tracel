use alloc::boxed::Box;
use core::fmt;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use futures::channel::oneshot;
use futures::future::{AbortHandle, Abortable};

use crate::spawn::{MaybeSend, Spawn};

/// No result was delivered: the work was aborted, panicked, or its owner shut down first.
///
/// Every error a [`Task`] carries must be able to represent this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Aborted;

impl fmt::Display for Aborted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the operation was aborted before it produced a result")
    }
}

impl core::error::Error for Aborted {}

/// A handle to work that is already running.
///
/// Await it, [`block`](Task::block) on it at a native sync edge, or [`try_get`](Task::try_get) it
/// from a loop that must not suspend. Dropping the handle aborts the work;
/// [`detach`](Task::detach) releases it and lets the work finish unobserved.
///
/// `Task<T, E>` is `Send` whenever `T` and `E` are, whatever produces them.
#[must_use = "dropping the handle aborts the work; call `detach` to let it finish"]
pub struct Task<T, E = Aborted> {
    state: State<T, E>,
    abort: Option<AbortHandle>,
}

enum State<T, E> {
    Ready(Option<Result<T, E>>),
    Pending(oneshot::Receiver<Result<T, E>>),
}

/// The sending half of [`Task::channel`]: delivers the result to the waiting handle.
pub struct Reply<T, E> {
    tx: oneshot::Sender<Result<T, E>>,
}

impl<T, E> Task<T, E> {
    /// Creates a task that already holds `value`. Allocates no channel and spawns nothing.
    pub fn ready(value: T) -> Self {
        Self::from_result(Ok(value))
    }

    /// Creates a task that already failed with `error`.
    pub fn failed(error: E) -> Self {
        Self::from_result(Err(error))
    }

    /// Creates a task that already holds `result`.
    pub fn from_result(result: Result<T, E>) -> Self {
        Self {
            state: State::Ready(Some(result)),
            abort: None,
        }
    }

    /// Creates a pending task and the [`Reply`] that completes it.
    ///
    /// For work that runs somewhere the caller cannot spawn into, such as behind an actor's
    /// mailbox. Dropping the reply without sending completes the task with [`Aborted`]; dropping
    /// the task only tells the reply that nobody is waiting.
    pub fn channel() -> (Reply<T, E>, Self) {
        let (tx, rx) = oneshot::channel();
        let task = Self {
            state: State::Pending(rx),
            abort: None,
        };
        (Reply { tx }, task)
    }

    /// Starts `future` on `spawn` and returns the handle to its result.
    pub fn spawn<S, F>(spawn: &S, future: F) -> Self
    where
        S: Spawn + ?Sized,
        F: Future<Output = Result<T, E>> + MaybeSend + 'static,
        T: Send + 'static,
        E: Send + 'static,
    {
        let (reply, mut task) = Self::channel();
        let (handle, registration) = AbortHandle::new_pair();
        let work = Abortable::new(future, registration);

        spawn.spawn(Box::pin(async move {
            if let Ok(result) = work.await {
                reply.send(result);
            }
        }));

        task.abort = Some(handle);
        task
    }

    /// Takes the result if it has arrived, without waiting.
    pub fn try_get(&mut self) -> Option<Result<T, E>>
    where
        E: From<Aborted>,
    {
        match &mut self.state {
            State::Ready(slot) => slot.take(),
            State::Pending(rx) => match rx.try_recv() {
                Ok(Some(result)) => Some(result),
                Ok(None) => None,
                Err(oneshot::Canceled) => Some(Err(Aborted.into())),
            },
        }
    }

    /// Stops the work at its next suspension point, after which the task yields [`Aborted`].
    ///
    /// Only work started by [`Task::spawn`] can be stopped this way; a task from
    /// [`Task::channel`] is completed or abandoned by whoever holds its [`Reply`].
    pub fn abort(&mut self) {
        if let Some(abort) = self.abort.take() {
            abort.abort();
        }
    }

    /// Releases the handle and lets the work finish unobserved.
    pub fn detach(mut self) {
        self.abort = None;
    }

    /// Waits for the result on the current thread.
    ///
    /// Parks the thread on a channel, so it needs no runtime — and would stall an executor's
    /// thread if called from inside one; await there instead. Absent where no other thread can
    /// make progress in the meantime.
    #[cfg(all(feature = "std", not(target_arch = "wasm32")))]
    pub fn block(self) -> Result<T, E>
    where
        E: From<Aborted>,
    {
        futures::executor::block_on(self)
    }
}

impl<T, E> Unpin for Task<T, E> {}

impl<T, E: From<Aborted>> Future for Task<T, E> {
    type Output = Result<T, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match &mut self.get_mut().state {
            State::Ready(slot) => Poll::Ready(slot.take().expect("Task polled after completion")),
            State::Pending(rx) => match Pin::new(rx).poll(cx) {
                Poll::Ready(Ok(result)) => Poll::Ready(result),
                Poll::Ready(Err(oneshot::Canceled)) => Poll::Ready(Err(Aborted.into())),
                Poll::Pending => Poll::Pending,
            },
        }
    }
}

impl<T, E> Drop for Task<T, E> {
    fn drop(&mut self) {
        self.abort();
    }
}

impl<T, E> fmt::Debug for Task<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = match &self.state {
            State::Ready(Some(_)) => "ready",
            State::Ready(None) => "taken",
            State::Pending(_) => "pending",
        };
        f.debug_struct("Task").field("state", &state).finish()
    }
}

impl<T, E> Reply<T, E> {
    /// Delivers `result`. Nothing happens if the handle was already dropped.
    pub fn send(self, result: Result<T, E>) {
        let _ = self.tx.send(result);
    }
}

impl<T, E> fmt::Debug for Reply<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Reply").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    use futures::FutureExt;
    use futures::future::Shared;

    use super::*;
    use crate::spawn::ThreadSpawn;

    fn gate() -> (oneshot::Sender<()>, Shared<oneshot::Receiver<()>>) {
        let (tx, rx) = oneshot::channel();
        (tx, rx.shared())
    }

    fn set_within(flag: &AtomicBool, secs: u64) -> bool {
        let deadline = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < deadline {
            if flag.load(Ordering::SeqCst) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    struct DropFlag(Arc<AtomicBool>);

    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[test]
    fn given_ready_task_when_probed_then_value_is_available_immediately() {
        let mut task = Task::<_, Aborted>::ready(7);

        assert_eq!(task.try_get(), Some(Ok(7)));
        assert_eq!(task.try_get(), None);
    }

    #[test]
    fn given_reply_when_sent_then_blocking_returns_it() {
        let (reply, task) = Task::<u8, Aborted>::channel();

        reply.send(Ok(7));

        assert_eq!(task.block(), Ok(7));
    }

    #[test]
    fn given_reply_dropped_when_awaiting_then_result_is_aborted() {
        let (reply, task) = Task::<u8, Aborted>::channel();

        drop(reply);

        assert_eq!(task.block(), Err(Aborted));
    }

    #[test]
    fn given_pending_task_when_probed_then_try_get_is_none_until_it_completes() {
        let (release, gate) = gate();
        let mut task = Task::spawn(&ThreadSpawn, async move {
            gate.await.ok();
            Ok::<_, Aborted>(7)
        });

        assert_eq!(task.try_get(), None);
        release.send(()).unwrap();

        assert_eq!(task.block(), Ok(7));
    }

    #[test]
    fn given_task_when_awaited_from_another_future_then_it_resolves() {
        let task = Task::spawn(&ThreadSpawn, async { Ok::<_, Aborted>(7) });

        let result = futures::executor::block_on(async { task.await.map(|value| value + 1) });

        assert_eq!(result, Ok(8));
    }

    #[test]
    fn given_shared_spawner_when_spawning_through_the_arc_then_it_runs() {
        let spawn: Arc<dyn Spawn> = Arc::new(ThreadSpawn);

        let task = Task::spawn(&spawn, async { Ok::<_, Aborted>(7) });

        assert_eq!(task.block(), Ok(7));
    }

    #[test]
    fn given_detached_task_when_handle_is_released_then_work_still_completes() {
        let done = Arc::new(AtomicBool::new(false));
        let flag = done.clone();

        Task::spawn(&ThreadSpawn, async move {
            flag.store(true, Ordering::SeqCst);
            Ok::<_, Aborted>(())
        })
        .detach();

        assert!(set_within(&done, 5), "detached work did not run");
    }

    #[test]
    fn given_aborted_task_when_awaited_then_work_stopped_and_result_is_aborted() {
        let done = Arc::new(AtomicBool::new(false));
        let flag = done.clone();
        let (_release, gate) = gate();

        let mut task = Task::spawn(&ThreadSpawn, async move {
            gate.await.ok();
            flag.store(true, Ordering::SeqCst);
            Ok::<_, Aborted>(())
        });
        task.abort();

        assert_eq!(task.block(), Err(Aborted));
        assert!(
            !done.load(Ordering::SeqCst),
            "aborted work ran to completion"
        );
    }

    #[test]
    fn given_dropped_handle_then_work_is_dropped_at_its_next_suspension() {
        let done = Arc::new(AtomicBool::new(false));
        let flag = done.clone();
        let (_release, gate) = gate();
        let dropped = Arc::new(AtomicBool::new(false));
        let guard = DropFlag(dropped.clone());

        let task = Task::spawn(&ThreadSpawn, async move {
            let _guard = guard;
            gate.await.ok();
            flag.store(true, Ordering::SeqCst);
            Ok::<_, Aborted>(())
        });
        drop(task);

        assert!(set_within(&dropped, 5), "the work was not dropped");
        assert!(
            !done.load(Ordering::SeqCst),
            "aborted work ran to completion"
        );
    }

    #[test]
    fn given_caller_error_type_when_worker_vanishes_then_aborted_converts_into_it() {
        #[derive(Debug, PartialEq)]
        enum AppError {
            Gone,
        }
        impl From<Aborted> for AppError {
            fn from(_: Aborted) -> Self {
                Self::Gone
            }
        }
        let (reply, task) = Task::<u8, AppError>::channel();

        drop(reply);

        assert_eq!(task.block(), Err(AppError::Gone));
    }
}
