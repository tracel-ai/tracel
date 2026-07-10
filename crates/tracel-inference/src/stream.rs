use crate::{OutputWriter, output::OutputWriterError};

use crossbeam::channel as cb;

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

pub type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

pub enum StreamEvent<O> {
    Output(O),
    Error(BoxError),
    Done(Duration),
}

pub struct InferenceStream<O> {
    rx: cb::Receiver<StreamEvent<O>>,
    cancel: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl<O> InferenceStream<O>
where
    O: Send + Sync + 'static,
{
    /// Spawn a worker thread that runs `run` against a fresh streaming channel, returning the
    /// consumer side as an iterator of outputs.
    ///
    /// The worker writes each output into the channel as it is produced. Dropping the returned
    /// stream cancels the request and joins the worker.
    pub(crate) fn spawn<F>(run: F) -> Self
    where
        F: FnOnce(StreamingOutput<O>) + Send + 'static,
    {
        let (tx, rx) = cb::unbounded();
        let cancel = Arc::new(AtomicBool::new(false));
        let channel = StreamingOutput {
            tx,
            cancel: cancel.clone(),
        };
        let worker = thread::spawn(move || run(channel));

        Self {
            rx,
            cancel,
            worker: Some(worker),
        }
    }
}

impl<O> InferenceStream<O> {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }

    fn join(&mut self) {
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}

impl<O> Drop for InferenceStream<O> {
    fn drop(&mut self) {
        self.cancel();
        self.join();
    }
}

impl<O> Iterator for InferenceStream<O> {
    type Item = Result<O, BoxError>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.rx.recv().ok()? {
            StreamEvent::Output(o) => Some(Ok(o)),
            StreamEvent::Error(e) => Some(Err(e)),
            StreamEvent::Done(_) => None,
        }
    }
}

/// The [`OutputWriter`] backing an [`InferenceStream`]: outputs, errors, and completion
/// are forwarded over a channel to the consuming iterator, and a cancel flag lets the consumer stop
/// the worker by reporting [`OutputWriterError::Cancelled`] on the next write.
pub(crate) struct StreamingOutput<O> {
    tx: cb::Sender<StreamEvent<O>>,
    cancel: Arc<AtomicBool>,
}

impl<O> OutputWriter<O> for StreamingOutput<O>
where
    O: Send + Sync + 'static,
{
    fn write(&self, output: O) -> Result<(), OutputWriterError> {
        if self.cancel.load(Ordering::Acquire) {
            return Err(OutputWriterError::Cancelled);
        }

        self.tx
            .send(StreamEvent::Output(output))
            .map_err(|e| OutputWriterError::Unknown(Box::new(e)))
    }

    fn error(&self, error: BoxError) -> Result<(), OutputWriterError> {
        self.tx
            .send(StreamEvent::Error(error))
            .map_err(|e| OutputWriterError::Unknown(Box::new(e)))
    }

    fn finish(&self, duration: Duration) {
        let _ = self.tx.send(StreamEvent::Done(duration));
    }
}
