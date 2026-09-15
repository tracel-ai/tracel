//! How this environment drives the SDK's IO, one definition per target.

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use core::future::Future;
    use core::pin::Pin;
    use core::task::{Context, Poll};
    use std::io;
    use std::thread;

    use futures::Stream;
    use futures::channel::oneshot;
    use tokio::runtime::{Builder, Handle};

    /// A tokio runtime a backend drives its IO with: the one the caller is already inside, or
    /// one of its own on a thread of its own.
    ///
    /// Work that needs tokio — a reqwest call, a tokio socket — is
    /// [`attach`](Runtime::attach)ed to the runtime and can then be polled by any executor; a
    /// loop that owns a resource is [`spawn`](Runtime::spawn)ed on it. The runtime only drives
    /// the IO; nothing blocks on it. An owned runtime shuts down when this is dropped.
    pub struct Runtime {
        handle: Handle,
        shutdown: Option<oneshot::Sender<()>>,
    }

    impl Runtime {
        /// Borrows the runtime the calling thread is inside, or starts one of its own.
        pub fn acquire() -> io::Result<Self> {
            match Handle::try_current() {
                Ok(handle) => Ok(Self {
                    handle,
                    shutdown: None,
                }),
                Err(_) => Self::start(),
            }
        }

        /// Starts a single-threaded runtime on a thread of its own.
        ///
        /// Only the scheduler is enabled here; IO and timer drivers come with whichever tokio
        /// features the transports in the build turn on. The thread sits in `block_on`, which
        /// is what drives those drivers for work attached from other threads.
        pub fn start() -> io::Result<Self> {
            let runtime = Builder::new_current_thread().enable_all().build()?;
            let handle = runtime.handle().clone();
            let (shutdown, stopped) = oneshot::channel::<()>();
            thread::Builder::new()
                .name("tracel-runtime".into())
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

        /// Attaches `future` to this runtime: any executor may poll the result, and the IO it
        /// does is driven here.
        pub fn attach<F: Future>(&self, future: F) -> Attached<F> {
            Attached {
                handle: self.handle.clone(),
                inner: future,
            }
        }

        /// Attaches `stream` to this runtime; see [`attach`](Runtime::attach).
        pub fn attach_stream<S: Stream>(&self, stream: S) -> AttachedStream<S> {
            AttachedStream {
                handle: self.handle.clone(),
                inner: stream,
            }
        }

        /// Runs `future` on the runtime until it completes, for a loop that owns a resource.
        pub fn spawn<F>(&self, future: F)
        where
            F: Future<Output = ()> + Send + 'static,
        {
            self.handle.spawn(future);
        }
    }

    impl Drop for Runtime {
        fn drop(&mut self) {
            if let Some(shutdown) = self.shutdown.take() {
                let _ = shutdown.send(());
            }
        }
    }

    pin_project_lite::pin_project! {
        /// A future polled inside its runtime's context; see [`Runtime::attach`].
        pub struct Attached<F> {
            handle: Handle,
            #[pin]
            inner: F,
        }
    }

    impl<F: Future> Future for Attached<F> {
        type Output = F::Output;

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            let this = self.project();
            let _guard = this.handle.enter();
            this.inner.poll(cx)
        }
    }

    pin_project_lite::pin_project! {
        /// A stream polled inside its runtime's context; see [`Runtime::attach_stream`].
        pub struct AttachedStream<S> {
            handle: Handle,
            #[pin]
            inner: S,
        }
    }

    impl<S: Stream> Stream for AttachedStream<S> {
        type Item = S::Item;

        fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            let this = self.project();
            let _guard = this.handle.enter();
            this.inner.poll_next(cx)
        }
    }

    #[cfg(test)]
    mod tests {
        use std::time::{Duration, Instant};

        use futures::channel::oneshot;
        use tokio::runtime::Builder;

        use super::*;
        use crate::Job;

        #[test]
        fn given_no_runtime_on_the_thread_when_acquired_then_one_is_started_and_owned() {
            let runtime = Runtime::acquire().unwrap();

            assert!(runtime.shutdown.is_some());
        }

        #[test]
        fn given_a_runtime_on_the_thread_when_acquired_then_it_is_borrowed() {
            let host = Builder::new_current_thread().build().unwrap();

            let runtime = host.block_on(async { Runtime::acquire().unwrap() });

            assert!(runtime.shutdown.is_none());
            assert_eq!(runtime.handle().id(), host.handle().id());
        }

        #[test]
        fn given_attached_tokio_io_when_blocked_on_without_a_runtime_then_it_completes() {
            let runtime = Runtime::start().unwrap();
            let job = Job::<_, ()>::new(
                runtime.attach(async { tokio::task::spawn_blocking(|| 7).await.map_err(|_| ()) }),
            );

            assert_eq!(job.block(), Ok(7));
        }

        #[test]
        fn given_attached_work_when_polled_per_tick_from_a_plain_loop_then_it_completes() {
            let runtime = Runtime::start().unwrap();
            let mut job = Job::<_, ()>::new(runtime.attach(async {
                tokio::time::sleep(Duration::from_millis(20)).await;
                Ok(7)
            }));

            let deadline = Instant::now() + Duration::from_secs(5);
            let result = loop {
                if let Some(result) = job.try_poll() {
                    break result;
                }
                assert!(Instant::now() < deadline, "the job never completed");
                std::thread::sleep(Duration::from_millis(1));
            };

            assert_eq!(result, Ok(7));
        }

        #[test]
        fn given_a_loop_spawned_on_the_runtime_then_it_runs_there_while_the_caller_goes_on() {
            let runtime = Runtime::start().unwrap();
            let (tx, rx) = oneshot::channel();

            runtime.spawn(async move {
                let _ = tx.send(std::thread::current().name().map(std::string::String::from));
            });

            let name = futures::executor::block_on(rx).unwrap();
            assert_eq!(name.as_deref(), Some("tracel-runtime"));
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use core::future::Future;
    use std::io;

    use futures::Stream;

    /// The browser's event loop, which drives every future and needs nothing owned.
    ///
    /// Attaching is the identity here: a `fetch` future is already pollable from anywhere.
    pub struct Runtime;

    /// A future attached to the browser is the future itself.
    pub type Attached<F> = F;

    /// A stream attached to the browser is the stream itself.
    pub type AttachedStream<S> = S;

    impl Runtime {
        /// There is nothing to acquire; the event loop is always there.
        pub fn acquire() -> io::Result<Self> {
            Ok(Self)
        }

        /// Hands `future` back unchanged; any executor on the event loop may poll it.
        pub fn attach<F: Future>(&self, future: F) -> Attached<F> {
            future
        }

        /// Hands `stream` back unchanged.
        pub fn attach_stream<S: Stream>(&self, stream: S) -> AttachedStream<S> {
            stream
        }

        /// Runs `future` on the event loop until it completes, for a loop that owns a resource.
        pub fn spawn<F>(&self, future: F)
        where
            F: Future<Output = ()> + 'static,
        {
            wasm_bindgen_futures::spawn_local(future);
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use browser::{Attached, AttachedStream, Runtime};
#[cfg(not(target_arch = "wasm32"))]
pub use native::{Attached, AttachedStream, Runtime};
