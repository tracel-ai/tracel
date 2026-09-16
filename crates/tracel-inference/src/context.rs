//! Thread-local ambient inference session, installed for the duration of a request.
//!
//! The ambient session is bound to a single thread and to a closure: it is pushed for
//! [`InferenceSession::run`](crate::InferenceSession::run) and popped when that returns, so it
//! can never outlive an `.await`. If `infer` spawns its own threads or tasks, capture the session
//! first and move a clone into the spawned work.

use std::cell::RefCell;

use crate::session::InferenceSession;

thread_local! {
    static CURRENT_SESSIONS: RefCell<Vec<InferenceSession>> = const { RefCell::new(Vec::new()) };
}

/// Runs `f` with `session` as the ambient session for the current thread.
pub(crate) fn with_session<T>(session: InferenceSession, f: impl FnOnce() -> T) -> T {
    CURRENT_SESSIONS.with(|sessions| sessions.borrow_mut().push(session));
    struct Pop;
    impl Drop for Pop {
        fn drop(&mut self) {
            CURRENT_SESSIONS.with(|sessions| {
                sessions.borrow_mut().pop();
            });
        }
    }
    let _pop = Pop;
    f()
}

/// The ambient session for the current thread, if any.
pub(crate) fn current_session() -> Option<InferenceSession> {
    CURRENT_SESSIONS.with(|sessions| sessions.borrow().last().cloned())
}
