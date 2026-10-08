//! The request that the job stop.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// What runs once a stop is requested.
type Action = Box<dyn FnOnce() + Send>;

/// The request that the job stop: requested once, by the first termination signal, when it runs
/// each action registered on it. Clones share the request.
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
    /// Requests the stop: runs each action registered, once. Requesting it again does nothing.
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

    /// Runs `action` once the stop is requested, at once when it already was.
    pub fn on_request(&self, action: Action) {
        let mut state = self.lock();
        if state.requested {
            drop(state);
            action();
        } else {
            state.actions.push(action);
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
