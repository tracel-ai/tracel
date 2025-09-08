use super::error::InferenceError;
use super::streaming::CancelToken;
use std::thread::JoinHandle;

/// Handle to a running inference job thread.
///
/// Provides access to the output stream (`stream`) plus cancellation (`cancel`) and
/// a `join` method to retrieve the final result (or error) once the handler terminates.
pub struct JobHandle<S> {
    pub id: String,
    pub stream: crossbeam::channel::Receiver<S>,
    cancel: CancelToken,
    join: Option<JoinHandle<Result<(), InferenceError>>>,
}

impl<S> JobHandle<S> {
    pub fn new(
        id: String,
        stream: crossbeam::channel::Receiver<S>,
        cancel: CancelToken,
        join: JoinHandle<Result<(), InferenceError>>,
    ) -> Self {
        Self {
            id,
            stream,
            cancel,
            join: Some(join),
        }
    }

    /// Cancel the running job. This will signal the job to stop processing as soon as possible.
    /// Note that this does not immediately kill the thread, but rather requests it to stop.
    /// The inference function has to use the `CancelToken` to check for cancellation.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// Wait for the job to finish and return the result.
    pub fn join(mut self) -> Result<(), InferenceError> {
        if let Some(join) = self.join.take() {
            let res = join.join();
            match res {
                Ok(r) => r,
                Err(e) => Err(InferenceError::ThreadPanicked(format!("{e:?}"))),
            }
        } else {
            Ok(())
        }
    }
}
