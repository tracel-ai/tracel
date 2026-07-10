//! Thread-local ambient inference session, installed on the per-request worker thread.
//!
//! The ambient session is bound to a single thread. If `infer` spawns its own threads or tasks,
//! capture the session first and move a clone into the spawned work.

use std::cell::RefCell;

use crate::session::InferenceSession;

thread_local! {
    static CURRENT_SESSIONS: RefCell<Vec<InferenceSession>> = const { RefCell::new(Vec::new()) };
}

/// RAII guard that pops the ambient session when dropped.
#[must_use = "the ambient session is cleared when the guard is dropped"]
pub struct SessionGuard {
    _private: (),
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        CURRENT_SESSIONS.with(|sessions| {
            sessions.borrow_mut().pop();
        });
    }
}

/// Push `session` as the ambient session for the current thread until the returned guard drops.
pub(crate) fn enter(session: InferenceSession) -> SessionGuard {
    CURRENT_SESSIONS.with(|sessions| sessions.borrow_mut().push(session));
    SessionGuard { _private: () }
}

/// The ambient session for the current thread, if any.
pub(crate) fn current_session() -> Option<InferenceSession> {
    CURRENT_SESSIONS.with(|sessions| sessions.borrow().last().cloned())
}
