use super::error::InferenceError;
use super::streaming::CancelToken;
use std::thread::JoinHandle;

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

    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    pub fn join(mut self) -> Result<(), InferenceError> {
        if let Some(join) = self.join.take() {
            Ok(join.join().unwrap_or_else(|e| {
                Err(InferenceError::ThreadPanicked(format!(
                    "Inference thread panicked: {e:?}"
                )))
            })?)
        } else {
            Ok(())
        }
    }
}
