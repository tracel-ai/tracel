#[cfg(not(target_arch = "wasm32"))]
use futures::future::BoxFuture;
#[cfg(target_arch = "wasm32")]
use futures::future::LocalBoxFuture;
#[cfg(not(target_arch = "wasm32"))]
use futures::stream::BoxStream;
#[cfg(target_arch = "wasm32")]
use futures::stream::LocalBoxStream;

/// `Send` on native targets, vacuous on wasm.
///
/// Bounds the futures handed to a [`Spawn`] and what they borrow, nothing else: handles and their
/// payloads are `Send` on every target.
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSend: Send {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send + ?Sized> MaybeSend for T {}

/// `Send` on native targets, vacuous on wasm.
///
/// Bounds the futures handed to a [`Spawn`] and what they borrow, nothing else: handles and their
/// payloads are `Send` on every target.
#[cfg(target_arch = "wasm32")]
pub trait MaybeSend {}
#[cfg(target_arch = "wasm32")]
impl<T: ?Sized> MaybeSend for T {}

/// `Sync` on native targets, vacuous on wasm.
///
/// Bounds what a spawned future borrows by shared reference.
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSync: Sync {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Sync + ?Sized> MaybeSync for T {}

/// `Sync` on native targets, vacuous on wasm.
///
/// Bounds what a spawned future borrows by shared reference.
#[cfg(target_arch = "wasm32")]
pub trait MaybeSync {}
#[cfg(target_arch = "wasm32")]
impl<T: ?Sized> MaybeSync for T {}

/// A boxed future: `Send` on native targets, thread-local on wasm.
///
/// The return type of an object-safe port method, so a port stays one trait on every target.
#[cfg(not(target_arch = "wasm32"))]
pub type DynFuture<'a, T> = BoxFuture<'a, T>;
/// A boxed future: `Send` on native targets, thread-local on wasm.
///
/// The return type of an object-safe port method, so a port stays one trait on every target.
#[cfg(target_arch = "wasm32")]
pub type DynFuture<'a, T> = LocalBoxFuture<'a, T>;

/// A boxed stream: `Send` on native targets, thread-local on wasm.
#[cfg(not(target_arch = "wasm32"))]
pub type DynStream<'a, T> = BoxStream<'a, T>;
/// A boxed stream: `Send` on native targets, thread-local on wasm.
#[cfg(target_arch = "wasm32")]
pub type DynStream<'a, T> = LocalBoxStream<'a, T>;

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

/// Runs each spawned future on its own OS thread.
///
/// Needs no runtime, which makes it the spawner for tests and for producers whose work does not
/// require one.
#[cfg(not(target_arch = "wasm32"))]
pub struct ThreadSpawn;

#[cfg(not(target_arch = "wasm32"))]
impl Spawn for ThreadSpawn {
    fn spawn(&self, future: SpawnedFuture) {
        std::thread::spawn(move || futures::executor::block_on(future));
    }
}

/// Runs spawned futures on the JavaScript event loop.
#[cfg(target_arch = "wasm32")]
pub struct BrowserSpawn;

#[cfg(target_arch = "wasm32")]
impl Spawn for BrowserSpawn {
    fn spawn(&self, future: SpawnedFuture) {
        wasm_bindgen_futures::spawn_local(future);
    }
}
