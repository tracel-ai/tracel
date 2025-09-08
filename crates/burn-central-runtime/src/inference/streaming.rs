use derive_more::Deref;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Error returned when emitting an item fails.
/// The item of type [T](EmitError::item) is returned to allow for potential retries.
#[derive(Debug, thiserror::Error)]
pub struct EmitError<T> {
    #[source]
    pub source: anyhow::Error,
    pub item: T,
}

/// The sending side of an output stream for inference outputs.
pub trait Emitter<T>: Send + Sync + 'static {
    fn emit(&self, item: T) -> Result<(), EmitError<T>>;
}

/// A token that can be used to cancel an ongoing inference job.
#[derive(Clone)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst)
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// An emitter that collects all emitted items into a vector.
pub struct CollectEmitter<T>(Mutex<Vec<T>>);

impl<T> CollectEmitter<T> {
    pub fn new() -> Self {
        Self(Mutex::new(Vec::new()))
    }

    pub fn into_inner(self) -> Vec<T> {
        self.0.into_inner().unwrap()
    }
}

impl<T> Default for CollectEmitter<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Send + 'static> Emitter<T> for CollectEmitter<T> {
    fn emit(&self, item: T) -> Result<(), EmitError<T>> {
        self.0.lock().unwrap().push(item);
        Ok(())
    }
}

pub struct SyncChannelEmitter<T> {
    tx: crossbeam::channel::Sender<T>,
}

impl<T: Send + 'static> SyncChannelEmitter<T> {
    pub fn new(tx: crossbeam::channel::Sender<T>) -> Self {
        Self { tx }
    }
}

impl<T: Send + 'static> Emitter<T> for SyncChannelEmitter<T> {
    fn emit(&self, item: T) -> Result<(), EmitError<T>> {
        match self.tx.try_send(item) {
            Ok(_) => Ok(()),
            Err(crossbeam::channel::TrySendError::Full(item)) => Err(EmitError {
                source: anyhow::anyhow!("Channel is full"),
                item,
            }),
            Err(crossbeam::channel::TrySendError::Disconnected(item)) => Err(EmitError {
                source: anyhow::anyhow!("Channel is disconnected"),
                item,
            }),
        }
    }
}

#[derive(Clone, Deref)]
pub struct OutStream<T> {
    emitter: Arc<dyn Emitter<T>>,
}

impl<T> OutStream<T> {
    pub fn new(emitter: Arc<dyn Emitter<T>>) -> Self {
        Self { emitter }
    }
}
