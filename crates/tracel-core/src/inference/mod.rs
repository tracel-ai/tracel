use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tracel_inference::observer::{InferenceWriterObserver, InferenceWriterStats};
use tracel_inference::{InferenceError, InferenceProvider, InferenceSession};

/// Default inference provider.
///
/// It opens one [`InferenceSession`] per request and scopes that request's telemetry. This first
/// cut is intentionally a **stub**: the session observer records request-completion stats locally
/// (via `tracing`). The seam to ship these as inference events over the client API key lives in
/// [`RequestTelemetryObserver::on_finish`].
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
        let observer = Arc::new(RequestTelemetryObserver {
            inference_name: name.to_string(),
            session_id: session_id.clone(),
        });
        Ok(InferenceSession::new(session_id, observer))
    }
}

/// Records per-request telemetry when a request's output writer is dropped.
struct RequestTelemetryObserver {
    inference_name: String,
    session_id: String,
}

impl InferenceWriterObserver for RequestTelemetryObserver {
    fn on_finish(&self, stats: &InferenceWriterStats) {
        // TODO: ship these stats as inference events over the client API key (fleet-shaped
        // payload minus device auth). For now they are recorded locally.
        tracing::info!(
            inference_name = %self.inference_name,
            session_id = %self.session_id,
            duration_ms = stats.duration.as_secs_f64() * 1_000.0,
            outputs = stats.outputs,
            errors = stats.errors,
            cancelled = stats.cancelled,
            "inference request finished"
        );
    }
}
