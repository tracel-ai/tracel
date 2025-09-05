use super::errors::InferenceError;
use derive_more::Deref;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::TrySendError;
use std::sync::{Arc, Mutex};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EmitControl {
    Continue,
    Stop,
}

pub trait Emitter<T>: Send + Sync + 'static {
    fn emit(&self, item: T) -> Result<EmitControl, InferenceError>;
    fn end(&self) -> Result<(), InferenceError> {
        Ok(())
    }
}

#[derive(Clone)]
pub struct CancelToken(pub(crate) Arc<AtomicBool>);

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

impl Default for CancelToken {
    fn default() -> Self {
        Self::new()
    }
}

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
    fn emit(&self, item: T) -> Result<EmitControl, InferenceError> {
        self.0.lock().unwrap().push(item);
        Ok(EmitControl::Continue)
    }
}

pub struct SyncChannelEmitter<T> {
    tx: std::sync::mpsc::SyncSender<T>,
}

impl<T: Send + 'static> SyncChannelEmitter<T> {
    pub fn new(tx: std::sync::mpsc::SyncSender<T>) -> Self {
        Self { tx }
    }
}

impl<T: Send + 'static> Emitter<T> for SyncChannelEmitter<T> {
    fn emit(&self, item: T) -> Result<EmitControl, InferenceError> {
        match self.tx.try_send(item) {
            Ok(_) => Ok(EmitControl::Continue),
            Err(TrySendError::Full(_)) => Ok(EmitControl::Stop),
            Err(TrySendError::Disconnected(_)) => Ok(EmitControl::Stop),
        }
    }
}

#[derive(Clone, Deref)]
pub struct OutStream<T> {
    pub(crate) emitter: Arc<dyn Emitter<T>>,
}

impl<T> OutStream<T> {
    pub fn new(emitter: Arc<dyn Emitter<T>>) -> Self {
        Self { emitter }
    }
}
