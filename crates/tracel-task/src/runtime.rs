use std::future::Future;
use std::io;
use std::thread;

use futures::channel::oneshot;
use tokio::runtime::{Builder, Handle};

use crate::spawn::{Spawn, SpawnedFuture};

/// A single-threaded tokio runtime on a thread of its own, owned by whoever started it.
///
/// Spawned futures run on that thread, and [`block_on`](TokioRuntime::block_on) lets a
/// synchronous caller wait on a future with the runtime driving its IO. The runtime shuts down
/// when this is dropped.
pub struct TokioRuntime {
    handle: Handle,
    shutdown: Option<oneshot::Sender<()>>,
}

impl TokioRuntime {
    /// Starts the runtime thread.
    ///
    /// Only the scheduler is enabled here; IO and timer drivers come with whichever tokio
    /// features the transports in the build turn on.
    pub fn start() -> io::Result<Self> {
        let runtime = Builder::new_current_thread().enable_all().build()?;
        let handle = runtime.handle().clone();
        let (shutdown, stopped) = oneshot::channel::<()>();
        thread::Builder::new()
            .name("tracel-runtime".to_string())
            .spawn(move || {
                runtime.block_on(async {
                    let _ = stopped.await;
                });
            })?;

        Ok(Self {
            handle,
            shutdown: Some(shutdown),
        })
    }

    /// The runtime's handle, for tokio-specific needs the seam does not cover.
    pub fn handle(&self) -> &Handle {
        &self.handle
    }

    /// Runs `future` to completion on the calling thread, with its IO driven by the runtime.
    ///
    /// Must not be called from inside the runtime itself; await there instead.
    pub fn block_on<F: Future>(&self, future: F) -> F::Output {
        self.handle.block_on(future)
    }
}

impl Spawn for TokioRuntime {
    fn spawn(&self, future: SpawnedFuture) {
        self.handle.spawn(future);
    }
}

impl Drop for TokioRuntime {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{Aborted, Task};

    #[test]
    fn given_owned_runtime_when_spawning_then_a_caller_without_a_runtime_can_block() {
        let runtime = TokioRuntime::start().unwrap();

        let task = Task::spawn(&runtime, async { Ok::<_, Aborted>(7) });

        assert_eq!(task.block(), Ok(7));
    }

    #[test]
    fn given_owned_runtime_when_a_future_is_blocked_on_then_it_completes_with_the_runtime_driving()
    {
        let runtime = Arc::new(TokioRuntime::start().unwrap());
        let inner = Arc::clone(&runtime);

        let value = runtime
            .block_on(async move { Task::spawn(&*inner, async { Ok::<_, Aborted>(7) }).await });

        assert_eq!(value, Ok(7));
    }
}
