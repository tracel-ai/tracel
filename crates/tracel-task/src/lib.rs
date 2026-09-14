#![deny(missing_docs)]

//! Effect handles for the Tracel SDK.
//!
//! An IO-bound operation returns a [`Task`] (one result) or a [`Streaming`] (many) that is
//! already running. The caller awaits it, blocks on it at a native sync edge, or polls it from a
//! loop that must not suspend — none of which requires running, or naming, an async runtime.
//! Whoever produces the result implements [`Spawn`] with the executor it owns.
//!
//! Handles are `Send` whenever their payloads are, regardless of the work behind them: only the
//! result crosses a handle, so a producer may hold state that cannot leave its thread.

mod spawn;
mod streaming;
mod task;

#[cfg(target_arch = "wasm32")]
pub use spawn::BrowserSpawn;
#[cfg(not(target_arch = "wasm32"))]
pub use spawn::ThreadSpawn;
pub use spawn::{DynFuture, DynStream, MaybeSend, MaybeSync, Spawn, SpawnedFuture};
#[cfg(not(target_arch = "wasm32"))]
pub use streaming::BlockingIter;
pub use streaming::{Closed, Streaming, StreamingSink, TrySendError};
pub use task::{AbortOnDrop, Aborted, Reply, Task};

/// Holds on every target: handles and their producing halves are `Send + Sync` for `Send`
/// payloads.
#[allow(dead_code)]
fn assert_handles_are_send_and_sync() {
    fn assert<T: Send + Sync>() {}

    assert::<Task<Vec<u8>, Aborted>>();
    assert::<AbortOnDrop<Vec<u8>, Aborted>>();
    assert::<Reply<Vec<u8>, Aborted>>();
    assert::<Streaming<Vec<u8>, Aborted>>();
    assert::<StreamingSink<Vec<u8>, Aborted>>();
}

/// Holds on every target: trait objects borrowed across a suspension point can carry the bounds.
#[allow(dead_code)]
fn assert_unsized_types_carry_the_bounds() {
    fn send<T: MaybeSend + ?Sized>() {}
    fn sync<T: MaybeSync + ?Sized>() {}

    send::<dyn std::any::Any + Send>();
    sync::<dyn std::any::Any + Sync>();
}

/// Holds on wasm: work that cannot leave its thread is still spawnable.
#[cfg(target_arch = "wasm32")]
#[allow(dead_code)]
fn assert_thread_local_work_is_spawnable(spawn: &dyn Spawn) {
    let local = std::rc::Rc::new(());
    Task::<(), Aborted>::spawn(spawn, async move {
        drop(local);
        Ok(())
    })
    .detach();
}
