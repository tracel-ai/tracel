#[cfg(not(any(target_arch = "wasm32", target_os = "none")))]
mod threaded {
    /// `Send` where work may move between threads, vacuous on single-threaded targets.
    ///
    /// Bounds the work inside a handle and what it borrows, nothing else: the results a handle
    /// yields are `Send` on every target.
    pub trait MaybeSend: Send {}
    impl<T: Send + ?Sized> MaybeSend for T {}

    /// `Sync` where work may move between threads, vacuous on single-threaded targets.
    ///
    /// Bounds what work borrows by shared reference.
    pub trait MaybeSync: Sync {}
    impl<T: Sync + ?Sized> MaybeSync for T {}

    /// A boxed future: `Send` where work may move between threads, thread-local otherwise.
    pub type DynFuture<'a, T> = futures::future::BoxFuture<'a, T>;

    /// A boxed stream: `Send` where work may move between threads, thread-local otherwise.
    pub type DynStream<'a, T> = futures::stream::BoxStream<'a, T>;
}

#[cfg(any(target_arch = "wasm32", target_os = "none"))]
mod local {
    /// `Send` where work may move between threads, vacuous on single-threaded targets.
    ///
    /// Bounds the work inside a handle and what it borrows, nothing else: the results a handle
    /// yields are `Send` on every target.
    pub trait MaybeSend {}
    impl<T: ?Sized> MaybeSend for T {}

    /// `Sync` where work may move between threads, vacuous on single-threaded targets.
    ///
    /// Bounds what work borrows by shared reference.
    pub trait MaybeSync {}
    impl<T: ?Sized> MaybeSync for T {}

    /// A boxed future: `Send` where work may move between threads, thread-local otherwise.
    pub type DynFuture<'a, T> = futures::future::LocalBoxFuture<'a, T>;

    /// A boxed stream: `Send` where work may move between threads, thread-local otherwise.
    pub type DynStream<'a, T> = futures::stream::LocalBoxStream<'a, T>;
}

#[cfg(any(target_arch = "wasm32", target_os = "none"))]
pub use local::*;
#[cfg(not(any(target_arch = "wasm32", target_os = "none")))]
pub use threaded::*;
