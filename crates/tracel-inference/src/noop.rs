use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::InferenceError;
use crate::provider::InferenceProvider;
use crate::session::InferenceSession;
use crate::sink::NoopSink;

/// An inference provider that ships nothing: its sessions record into a [`NoopSink`], so
/// per-request statistics, metrics and logs are discarded.
#[derive(Debug, Default)]
pub struct NoopInferenceProvider {
    request_counter: AtomicU64,
}

impl NoopInferenceProvider {
    /// Create a provider whose sessions discard what they record.
    pub fn new() -> Self {
        Self::default()
    }
}

impl InferenceProvider for NoopInferenceProvider {
    fn create_session(&self, name: &str) -> Result<InferenceSession, InferenceError> {
        let n = self.request_counter.fetch_add(1, Ordering::Relaxed);
        let session_id = format!("{name}/{n}");
        Ok(InferenceSession::new(session_id, Arc::new(NoopSink))
            .with_attributes([("inference_name", name.to_string())]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_session_has_its_own_id() {
        let provider = NoopInferenceProvider::new();

        let first = provider.create_session("wordtok").unwrap();
        let second = provider.create_session("wordtok").unwrap();

        assert_eq!(first.id().as_str(), "wordtok/0");
        assert_eq!(second.id().as_str(), "wordtok/1");
    }
}
