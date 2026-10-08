//! The request that a job stop.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// What runs once a stop is requested.
type Action = Box<dyn FnOnce() + Send>;

/// The request that a job stop, which its transport makes: on a termination signal for a process,
/// on a cancel from its server for another. Requested once, when it runs each action registered on
/// it. Clones share the request.
#[derive(Clone, Default)]
pub struct Stop {
    state: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    requested: bool,
    actions: Vec<Action>,
}

impl Stop {
    /// A stop not yet requested.
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests the stop: runs each action registered, once, on this thread. Requesting it again
    /// does nothing.
    pub fn request(&self) {
        let actions = {
            let mut state = self.lock();
            if state.requested {
                return;
            }
            state.requested = true;
            std::mem::take(&mut state.actions)
        };
        for action in actions {
            action();
        }
    }

    /// Whether the stop was requested.
    pub fn is_requested(&self) -> bool {
        self.lock().requested
    }

    /// Runs `action` once the stop is requested, on the thread that requests it, or at once when
    /// it already was.
    pub fn on_request(&self, action: impl FnOnce() + Send + 'static) {
        let mut state = self.lock();
        if state.requested {
            drop(state);
            action();
        } else {
            state.actions.push(Box::new(action));
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn counting(count: &Arc<AtomicUsize>) -> Action {
        let count = count.clone();
        Box::new(move || {
            count.fetch_add(1, Ordering::SeqCst);
        })
    }

    #[test]
    fn each_action_runs_once_when_the_stop_is_requested() {
        let stop = Stop::default();
        let count = Arc::new(AtomicUsize::new(0));
        stop.on_request(counting(&count));
        stop.clone().on_request(counting(&count));

        let before = (stop.is_requested(), count.load(Ordering::SeqCst));
        stop.request();
        stop.request();

        assert_eq!(before, (false, 0));
        assert!(stop.is_requested());
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn an_action_registered_once_the_stop_was_requested_runs_at_once() {
        let stop = Stop::default();
        let count = Arc::new(AtomicUsize::new(0));
        stop.request();

        stop.on_request(counting(&count));

        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
}
