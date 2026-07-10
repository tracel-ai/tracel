use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tracel_inference::sink::NoopSink;
use tracel_inference::{InferenceError, InferenceProvider, InferenceSession};

mod cloud;

pub use cloud::CloudInferenceProvider;

/// Local inference provider for offline execution: it ships nothing (sessions record into a
/// [`NoopSink`], so per-request stats and any metrics/logs are discarded). See
/// [`CloudInferenceProvider`] to ship.
#[derive(Default)]
pub struct DefaultInferenceProvider {
    request_counter: AtomicU64,
}

impl DefaultInferenceProvider {
    pub fn new() -> Self {
        Self::default()
    }
}

impl InferenceProvider for DefaultInferenceProvider {
    fn create_session(&self, name: &str) -> Result<InferenceSession, InferenceError> {
        let n = self.request_counter.fetch_add(1, Ordering::Relaxed);
        let session_id = format!("{name}/{n}");
        Ok(InferenceSession::new(session_id, Arc::new(NoopSink))
            .with_attributes([("inference_name", name.to_string())]))
    }
}
