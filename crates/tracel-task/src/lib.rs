#![no_std]
#![deny(missing_docs)]

//! Effect handles for the Tracel SDK.
//!
//! An IO-bound operation is handed back as work the caller decides how to drive. A [`Job`] has
//! not started: await it on any executor, block on it at a native sync edge, or spawn it and get
//! a [`Task`] — a handle to work already running, which can also be polled from a loop that must
//! not suspend. [`Streaming`] is the running handle's multi-item twin. None of this requires
//! running, or naming, an async runtime: whoever produces results implements [`Spawn`] with the
//! executor it owns.
//!
//! Handles are `Send` whenever their payloads are, regardless of the work behind them: only the
//! result crosses a handle, so a producer may hold state that cannot leave its thread.
//!
//! The crate is `no_std` with `alloc`. The `std` feature adds the blocking entry points
//! ([`Job::block`], [`Task::block`], [`Streaming::blocking_iter`]) and [`ThreadSpawn`], on targets
//! where another thread can make progress while one is parked; the `tokio` feature adds
//! [`TokioRuntime`].

extern crate alloc;
#[cfg(any(feature = "std", test))]
extern crate std;

mod job;
#[cfg(feature = "tokio")]
mod runtime;
mod spawn;
mod streaming;
mod task;

pub use job::Job;
#[cfg(feature = "tokio")]
pub use runtime::TokioRuntime;
#[cfg(target_arch = "wasm32")]
pub use spawn::BrowserSpawn;
#[cfg(all(feature = "std", not(target_arch = "wasm32")))]
pub use spawn::ThreadSpawn;
pub use spawn::{DynFuture, DynStream, MaybeSend, MaybeSync, Spawn, SpawnedFuture};
#[cfg(all(feature = "std", not(target_arch = "wasm32")))]
pub use streaming::BlockingIter;
pub use streaming::{Closed, Streaming, StreamingSink, TrySendError};
pub use task::{Aborted, Reply, Task};

/// Holds on every target: handles and their producing halves are `Send + Sync` for `Send`
/// payloads.
#[allow(dead_code)]
fn assert_handles_are_send_and_sync() {
    fn assert<T: Send + Sync>() {}

    assert::<Task<alloc::vec::Vec<u8>, Aborted>>();
    assert::<Reply<alloc::vec::Vec<u8>, Aborted>>();
    assert::<Streaming<alloc::vec::Vec<u8>, Aborted>>();
    assert::<StreamingSink<alloc::vec::Vec<u8>, Aborted>>();
}

/// Holds on every target: trait objects borrowed across a suspension point can carry the bounds.
#[allow(dead_code)]
fn assert_unsized_types_carry_the_bounds() {
    fn send<T: MaybeSend + ?Sized>() {}
    fn sync<T: MaybeSync + ?Sized>() {}

    send::<dyn core::any::Any + Send>();
    sync::<dyn core::any::Any + Sync>();
}

/// Holds on single-threaded targets: work that cannot leave its thread is still spawnable and
/// still composes into a job.
#[cfg(any(target_arch = "wasm32", target_os = "none"))]
#[allow(dead_code)]
fn assert_thread_local_work_is_spawnable(spawn: &dyn Spawn) {
    let local = alloc::rc::Rc::new(());
    let job = Job::<(), Aborted>::new(async move {
        drop(local);
        Ok(())
    });
    job.spawn_on(spawn).detach();
}
