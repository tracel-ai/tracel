use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

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

impl std::error::Error for Aborted {}

/// A handle to work that is already running.
///
/// Await it, [`block`](Task::block) on it at a native sync edge, or [`try_get`](Task::try_get) it
/// from a loop that must not suspend. Dropping the handle lets the work finish unobserved;
/// [`abort`](Task::abort) stops it.
///
/// `Task<T, E>` is `Send` whenever `T` and `E` are, whatever produces them.
#[must_use = "the work is already running; call `detach` to release the handle deliberately"]
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
    /// mailbox. Dropping the reply without sending completes the task with [`Aborted`].
    pub fn channel() -> (Reply<T, E>, Self) {
        let (tx, rx) = oneshot::channel();
        let task = Self {
            state: State::Pending(rx),
            abort: None,
        };
        (Reply { tx }, task)
    }

    /// Starts `future` on `spawn` and returns the handle to its result.
    pub fn spawn<F>(spawn: &dyn Spawn, future: F) -> Self
    where
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

    /// Runs `work` on `spawn` where blocking is allowed and returns the handle to its result.
    ///
    /// Blocking work cannot be aborted; [`Task::abort`] has no effect on it.
    pub fn spawn_blocking<F>(spawn: &dyn Spawn, work: F) -> Self
    where
        F: FnOnce() -> Result<T, E> + Send + 'static,
        T: Send + 'static,
        E: Send + 'static,
    {
        let (reply, task) = Self::channel();
        spawn.spawn_blocking(Box::new(move || reply.send(work())));
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
    pub fn detach(self) {}

    /// Makes dropping the handle abort the work, for work that is worthless once nobody waits
    /// for it.
    pub fn abort_on_drop(self) -> AbortOnDrop<T, E> {
        AbortOnDrop { task: self }
    }

    /// Waits for the result on the current thread.
    ///
    /// Native only. Parks the thread on a channel, so it needs no runtime — and would stall an
    /// executor's thread if called from inside one; await there instead.
    #[cfg(not(target_arch = "wasm32"))]
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

/// A [`Task`] whose work is aborted when the handle is dropped; see [`Task::abort_on_drop`].
#[must_use = "dropping this handle aborts the work"]
pub struct AbortOnDrop<T, E = Aborted> {
    task: Task<T, E>,
}

impl<T, E> AbortOnDrop<T, E> {
    /// Takes the result if it has arrived, without waiting.
    pub fn try_get(&mut self) -> Option<Result<T, E>>
    where
        E: From<Aborted>,
    {
        self.task.try_get()
    }
}

impl<T, E: From<Aborted>> Future for AbortOnDrop<T, E> {
    type Output = Result<T, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.get_mut().task).poll(cx)
    }
}

impl<T, E> Drop for AbortOnDrop<T, E> {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl<T, E> fmt::Debug for AbortOnDrop<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AbortOnDrop")
            .field("task", &self.task)
            .finish()
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
    fn given_abort_on_drop_handle_when_dropped_then_work_stops() {
        let done = Arc::new(AtomicBool::new(false));
        let flag = done.clone();
        let (_release, gate) = gate();
        let stopped = Arc::new(AtomicBool::new(false));
        let stop_flag = stopped.clone();
        struct DropFlag(Arc<AtomicBool>);
        impl Drop for DropFlag {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }

        let guard = DropFlag(stop_flag);
        let handle = Task::spawn(&ThreadSpawn, async move {
            let _guard = guard;
            gate.await.ok();
            flag.store(true, Ordering::SeqCst);
            Ok::<_, Aborted>(())
        })
        .abort_on_drop();
        drop(handle);

        assert!(set_within(&stopped, 5), "the work was not dropped");
        assert!(
            !done.load(Ordering::SeqCst),
            "aborted work ran to completion"
        );
    }

    #[test]
    fn given_abort_on_drop_handle_when_awaited_then_result_arrives_as_usual() {
        let handle = Task::spawn(&ThreadSpawn, async { Ok::<_, Aborted>(7) }).abort_on_drop();

        assert_eq!(futures::executor::block_on(handle), Ok(7));
    }

    #[test]
    fn given_blocking_work_when_spawned_then_its_result_arrives() {
        let task = Task::spawn_blocking(&ThreadSpawn, || Ok::<_, Aborted>(7));

        assert_eq!(task.block(), Ok(7));
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
