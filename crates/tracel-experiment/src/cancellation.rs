use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// A task or object that can participate in experiment cancellation propagation.
///
/// Implementations should make [`Self::cancel`] idempotent and thread-safe.
pub trait Cancellable: Send + Sync {
    /// Request cancellation.
    fn cancel(&self);

    /// Return `true` once cancellation has been observed.
    fn is_cancelled(&self) -> bool;
}

type CancellableRef = Arc<dyn Cancellable>;

/// Shareable cancellation token used by experiment runs and their children.
///
/// Cancelling a token also cancels every child that has been linked to it.
///
/// # Example
///
/// ```no_run
/// use tracel_experiment::CancelToken;
///
/// let parent = CancelToken::new();
/// let child = parent.linked(CancelToken::new());
///
/// assert!(!child.is_cancelled());
/// parent.cancel();
/// assert!(child.is_cancelled());
/// ```
#[derive(Clone, Default)]
pub struct CancelToken {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for CancelToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CancelToken")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

#[derive(Default)]
struct Inner {
    cancelled: AtomicBool,
    children: Mutex<Vec<CancellableRef>>,
}

impl CancelToken {
    /// Create a new uncancelled token with no linked children.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                cancelled: AtomicBool::new(false),
                children: Mutex::new(Vec::new()),
            }),
        }
    }

    /// Return `true` once this token has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Acquire)
    }

    /// Link a child so it is cancelled when this token is cancelled.
    ///
    /// If this token is already cancelled, the child is cancelled immediately.
    pub fn link<T: Cancellable + 'static>(&self, child: T) {
        let child = Arc::new(child);
        if self.is_cancelled() {
            child.cancel();
            return;
        }

        let mut children = self.inner.children.lock().unwrap();

        if self.is_cancelled() {
            drop(children);
            child.cancel();
            return;
        }

        children.push(child);
    }

    /// Cancel this token and all currently-linked children.
    pub fn cancel(&self) {
        if self.inner.cancelled.swap(true, Ordering::AcqRel) {
            return;
        }

        let children = {
            let mut children = self.inner.children.lock().unwrap();
            std::mem::take(&mut *children)
        };

        for c in children {
            c.cancel();
        }
    }

    /// Create a default child, link it to this token, and return it.
    pub fn into_linked<T: Cancellable + Default + Clone + 'static>(&self) -> T {
        let merged = T::default();
        self.link(merged.clone());
        merged
    }

    /// Link an existing child to this token and return it unchanged.
    pub fn linked<T: Cancellable + Clone + 'static>(&self, child: T) -> T {
        self.link(child.clone());
        child
    }
}

impl Cancellable for CancelToken {
    fn cancel(&self) {
        CancelToken::cancel(self)
    }
    fn is_cancelled(&self) -> bool {
        self.is_cancelled()
    }
}

impl<T: Cancellable> Cancellable for Arc<T> {
    fn cancel(&self) {
        self.as_ref().cancel();
    }
    fn is_cancelled(&self) -> bool {
        self.as_ref().is_cancelled()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Barrier;
    use std::sync::atomic::AtomicUsize;
    use std::thread;

    use super::*;

    #[derive(Default)]
    struct CountsCancels(AtomicUsize);

    impl CountsCancels {
        fn count(&self) -> usize {
            self.0.load(Ordering::Acquire)
        }
    }

    impl Cancellable for CountsCancels {
        fn cancel(&self) {
            self.0.fetch_add(1, Ordering::AcqRel);
        }

        fn is_cancelled(&self) -> bool {
            self.count() > 0
        }
    }

    fn linked_child(token: &CancelToken) -> Arc<CountsCancels> {
        let child = Arc::new(CountsCancels::default());
        token.link(child.clone());
        child
    }

    #[test]
    fn cancelling_a_token_cancels_every_child_linked_to_it() {
        let token = CancelToken::new();
        let children = [linked_child(&token), linked_child(&token)];

        token.cancel();

        assert!(children.iter().all(|child| child.is_cancelled()));
    }

    #[test]
    fn cancelling_a_token_twice_cancels_its_child_once() {
        let token = CancelToken::new();
        let child = linked_child(&token);

        token.cancel();
        token.cancel();

        assert_eq!(child.count(), 1);
    }

    #[test]
    fn cancelling_a_token_from_two_threads_at_once_cancels_its_child_once() {
        let token = CancelToken::new();
        let child = linked_child(&token);
        let barrier = Barrier::new(2);

        thread::scope(|scope| {
            for _ in 0..2 {
                scope.spawn(|| {
                    barrier.wait();
                    token.cancel();
                });
            }
        });

        assert_eq!(child.count(), 1);
    }

    #[test]
    fn a_child_linked_to_a_cancelled_token_is_cancelled() {
        let token = CancelToken::new();
        token.cancel();

        let child = linked_child(&token);

        assert!(child.is_cancelled());
    }
}
