#![no_std]
#![deny(missing_docs)]

//! Effect handles for the Tracel SDK.
//!
//! An IO-bound operation is handed back as a [`Job`] the caller drives: await it on any
//! executor, block on it at a native sync edge, poll it once per tick from a loop that must not
//! suspend, or spawn it on whatever the caller already has. [`Streaming`] is its multi-item
//! twin. None of this requires running, or naming, an async runtime.
//!
//! How IO is driven is the environment's business, and [`Runtime`] is that environment, one
//! definition per target: natively (under the `tokio` feature) a tokio runtime borrowed from
//! the caller or owned on a thread, to which a backend [`attach`](Runtime::attach)es work so
//! that any executor can poll it; in the browser, the event loop, where attaching is the
//! identity. An embedded backend brings its own.
//!
//! The crate is `no_std` with `alloc`. The `std` feature adds the blocking entry points
//! ([`Job::block`], [`Streaming::blocking_iter`]) on targets where another thread can make
//! progress while one is parked.

extern crate alloc;
#[cfg(any(feature = "std", test))]
extern crate std;

mod bounds;
mod job;
#[cfg(any(feature = "tokio", target_arch = "wasm32"))]
mod runtime;
mod streaming;

pub use bounds::{DynFuture, DynStream, MaybeSend, MaybeSync};
pub use job::Job;
#[cfg(any(feature = "tokio", target_arch = "wasm32"))]
pub use runtime::{Attached, AttachedStream, Runtime};
#[cfg(all(feature = "std", not(target_arch = "wasm32")))]
pub use streaming::BlockingIter;
pub use streaming::{Closed, Streaming, StreamingSink, TrySendError};

/// Holds where work may move between threads: handles and their producing halves are
/// `Send + Sync` for `Send` payloads. On single-threaded targets a handle may hold thread-local
/// work and is not expected to be.
#[cfg(not(any(target_arch = "wasm32", target_os = "none")))]
#[allow(dead_code)]
fn assert_handles_are_send_and_sync() {
    fn send<T: Send>() {}
    fn sync<T: Sync>() {}

    send::<Job<alloc::vec::Vec<u8>, ()>>();
    send::<Streaming<alloc::vec::Vec<u8>, ()>>();
    sync::<StreamingSink<alloc::vec::Vec<u8>, ()>>();
}

/// Holds on every target: trait objects borrowed across a suspension point can carry the bounds.
#[allow(dead_code)]
fn assert_unsized_types_carry_the_bounds() {
    fn send<T: MaybeSend + ?Sized>() {}
    fn sync<T: MaybeSync + ?Sized>() {}

    send::<dyn core::any::Any + Send>();
    sync::<dyn core::any::Any + Sync>();
}

/// Holds on single-threaded targets: work that cannot leave its thread still composes into a
/// job.
#[cfg(any(target_arch = "wasm32", target_os = "none"))]
#[allow(dead_code)]
fn assert_thread_local_work_composes() -> Job<(), ()> {
    let local = alloc::rc::Rc::new(());
    Job::new(async move {
        drop(local);
        Ok(())
    })
}
