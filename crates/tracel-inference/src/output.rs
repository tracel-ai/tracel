use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use crate::observer::{InferenceOutputObserver, InferenceOutputStats};

/// Errors that can occur when writing to an inference channel.
#[derive(Debug, thiserror::Error)]
pub enum OutputWriterError {
    #[error("inference was cancelled")]
    Cancelled,
    #[error("unknown error: {0}")]
    Unknown(Box<dyn std::error::Error + Send + Sync>),
}

/// Communication channel for an inference task, allowing the app to send outputs and errors back to the session.
pub struct InferenceOutput<O> {
    writer: Box<dyn OutputWriter<O>>,
    instant: std::time::Instant,
    observer: Option<Arc<dyn InferenceOutputObserver>>,
    outputs: AtomicUsize,
    errors: AtomicUsize,
    cancelled: AtomicBool,
    finished: AtomicBool,
}

impl<O> InferenceOutput<O> {
    pub(crate) fn new(writer: Box<dyn OutputWriter<O>>) -> Self {
        Self {
            writer,
            instant: std::time::Instant::now(),
            observer: None,
            outputs: AtomicUsize::new(0),
            errors: AtomicUsize::new(0),
            cancelled: AtomicBool::new(false),
            finished: AtomicBool::new(false),
        }
    }

    pub(crate) fn from_writer<C>(writer: C) -> Self
    where
        C: OutputWriter<O> + 'static,
    {
        Self::new(Box::new(writer))
    }

    pub fn with_observer(mut self, observer: Arc<dyn InferenceOutputObserver>) -> Self {
        self.observer = Some(observer);
        self
    }

    /// Respond with an output item. This can be called multiple times to emit multiple items.
    pub fn write(&self, output: O) -> Result<(), OutputWriterError> {
        match self.writer.write(output) {
            Ok(()) => {
                self.outputs.fetch_add(1, Ordering::Relaxed);
                if let Some(ref observer) = self.observer {
                    observer.on_write();
                }
                Ok(())
            }
            Err(err) => {
                if matches!(&err, OutputWriterError::Cancelled) {
                    self.cancelled.store(true, Ordering::Release);
                    if let Some(ref observer) = self.observer {
                        observer.on_cancelled();
                    }
                }
                Err(err)
            }
        }
    }

    /// Signal an error on the inference.
    pub fn error<E>(&self, error: E) -> Result<(), OutputWriterError>
    where
        E: Into<Box<dyn std::error::Error + Send + Sync>>,
    {
        match self.writer.error(error.into()) {
            Ok(()) => {
                self.errors.fetch_add(1, Ordering::Relaxed);
                if let Some(ref observer) = self.observer {
                    observer.on_error();
                }
                Ok(())
            }
            Err(err) => {
                if matches!(&err, OutputWriterError::Cancelled) {
                    self.cancelled.store(true, Ordering::Release);
                    if let Some(ref observer) = self.observer {
                        observer.on_cancelled();
                    }
                }
                Err(err)
            }
        }
    }

    fn finish(&self) {
        let duration = self.instant.elapsed();
        self.writer.finish(duration);

        if self.finished.swap(true, Ordering::AcqRel) {
            return;
        }

        if let Some(ref observer) = self.observer {
            observer.on_finish(&InferenceOutputStats {
                duration,
                outputs: self.outputs.load(Ordering::Acquire),
                errors: self.errors.load(Ordering::Acquire),
                cancelled: self.cancelled.load(Ordering::Acquire),
            });
        }
    }
}

/// When the `InferenceOutput` is dropped, it signals that the inference has finished, allowing the writer to perform any necessary cleanup or finalization.
impl<O> Drop for InferenceOutput<O> {
    fn drop(&mut self) {
        self.finish();
    }
}

/// Sink for an inference's typed outputs and errors.
///
/// Each transport implements this to encode and deliver items as the inference produces them.
pub trait OutputWriter<O> {
    /// Write an output item to the writer. This can be called multiple times to emit multiple items.
    fn write(&self, output: O) -> Result<(), OutputWriterError>;
    /// Signal an error on the inference, which will be sent back to the session.
    fn error(
        &self,
        error: Box<dyn std::error::Error + Send + Sync>,
    ) -> Result<(), OutputWriterError>;
    /// Called when the `InferenceOutput` is dropped, allowing the writer to perform any necessary cleanup or finalization.
    fn finish(&self, duration: std::time::Duration);
}
