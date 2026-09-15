use alloc::sync::Arc;

#[cfg(not(any(target_arch = "wasm32", target_os = "none")))]
mod threaded {
    /// `Send` where work may move between threads, vacuous on single-threaded targets.
    ///
    /// Bounds the futures handed to a [`Spawn`](super::Spawn) and what they borrow, nothing
    /// else: handles and their payloads are `Send` on every target.
    pub trait MaybeSend: Send {}
    impl<T: Send + ?Sized> MaybeSend for T {}

    /// `Sync` where work may move between threads, vacuous on single-threaded targets.
    ///
    /// Bounds what a spawned future borrows by shared reference.
    pub trait MaybeSync: Sync {}
    impl<T: Sync + ?Sized> MaybeSync for T {}

    /// A boxed future: `Send` where work may move between threads, thread-local otherwise.
    ///
    /// Names the future a port implementation or a transport works with below the seam; a port
    /// signature returns a handle instead, so that no runtime requirement hides in it.
    pub type DynFuture<'a, T> = futures::future::BoxFuture<'a, T>;

    /// A boxed stream: `Send` where work may move between threads, thread-local otherwise.
    pub type DynStream<'a, T> = futures::stream::BoxStream<'a, T>;

    /// Runs each spawned future on its own OS thread.
    ///
    /// Needs no runtime, which makes it the spawner for tests, and for a [`Job`](crate::Job)
    /// whose in-memory work should stay off an executor's thread.
    #[cfg(feature = "std")]
    pub struct ThreadSpawn;

    #[cfg(feature = "std")]
    impl super::Spawn for ThreadSpawn {
        fn spawn(&self, future: super::SpawnedFuture) {
            std::thread::spawn(move || futures::executor::block_on(future));
        }
    }
}

#[cfg(any(target_arch = "wasm32", target_os = "none"))]
mod local {
    /// `Send` where work may move between threads, vacuous on single-threaded targets.
    ///
    /// Bounds the futures handed to a [`Spawn`](super::Spawn) and what they borrow, nothing
    /// else: handles and their payloads are `Send` on every target.
    pub trait MaybeSend {}
    impl<T: ?Sized> MaybeSend for T {}

    /// `Sync` where work may move between threads, vacuous on single-threaded targets.
    ///
    /// Bounds what a spawned future borrows by shared reference.
    pub trait MaybeSync {}
    impl<T: ?Sized> MaybeSync for T {}

    /// A boxed future: `Send` where work may move between threads, thread-local otherwise.
    ///
    /// Names the future a port implementation or a transport works with below the seam; a port
    /// signature returns a handle instead, so that no runtime requirement hides in it.
    pub type DynFuture<'a, T> = futures::future::LocalBoxFuture<'a, T>;

    /// A boxed stream: `Send` where work may move between threads, thread-local otherwise.
    pub type DynStream<'a, T> = futures::stream::LocalBoxStream<'a, T>;

    /// Runs spawned futures on the JavaScript event loop.
    #[cfg(target_arch = "wasm32")]
    pub struct BrowserSpawn;

    #[cfg(target_arch = "wasm32")]
    impl super::Spawn for BrowserSpawn {
        fn spawn(&self, future: super::SpawnedFuture) {
            wasm_bindgen_futures::spawn_local(future);
        }
    }
}

#[cfg(any(target_arch = "wasm32", target_os = "none"))]
pub use local::*;
#[cfg(not(any(target_arch = "wasm32", target_os = "none")))]
pub use threaded::*;

/// A boxed future ready to be spawned.
pub type SpawnedFuture = DynFuture<'static, ()>;

/// Runs futures to completion in the background.
///
/// The one runtime capability the handles need. Whoever owns an executor implements it and hands
/// it to the code that produces results.
pub trait Spawn: Send + Sync + 'static {
    /// Starts `future` and returns without waiting for it.
    fn spawn(&self, future: SpawnedFuture);
}

impl<S: Spawn + ?Sized> Spawn for Arc<S> {
    fn spawn(&self, future: SpawnedFuture) {
        (**self).spawn(future);
    }
}
