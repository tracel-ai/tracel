//! Route `tracing` events into the ambient inference session.
//!
//! Install [`inference_log_layer`] on your subscriber; events emitted while a session is ambient
//! are recorded as scoped logs, with event and enclosing span fields folded into the metadata.

use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

mod layer;
mod visitor;

pub use layer::InferenceTracingLogLayer;

/// Create a layer that forwards `tracing` events to the ambient inference session.
pub fn inference_log_layer() -> InferenceTracingLogLayer {
    InferenceTracingLogLayer
}

/// Best-effort install of a default subscriber that includes inference log forwarding.
///
/// Returns `true` when a subscriber was installed and `false` when one already was.
pub fn try_init_tracing_subscriber() -> bool {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(env_filter)
        .with(inference_log_layer())
        .try_init()
        .is_ok()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tracing_subscriber::layer::SubscriberExt;

    use super::*;
    use crate::InferenceSession;
    use crate::observer::InferenceWriterObserver;
    use crate::sink::{InferenceSink, LogSample, MetricSample};

    #[derive(Default)]
    struct RecordingSink {
        logs: Mutex<Vec<LogSample>>,
        metrics: Mutex<Vec<MetricSample>>,
    }

    impl InferenceSink for RecordingSink {
        fn record_metric(&self, sample: MetricSample) {
            self.metrics.lock().unwrap().push(sample);
        }

        fn record_log(&self, sample: LogSample) {
            self.logs.lock().unwrap().push(sample);
        }
    }

    struct NoopObserver;
    impl InferenceWriterObserver for NoopObserver {}

    fn session(sink: Arc<RecordingSink>) -> InferenceSession {
        InferenceSession::new("req-1", Arc::new(NoopObserver), sink)
    }

    #[test]
    fn tracing_event_records_scoped_log_on_ambient_session() {
        let sink = Arc::new(RecordingSink::default());
        let session = session(sink.clone());
        let subscriber = tracing_subscriber::registry().with(inference_log_layer());

        tracing::subscriber::with_default(subscriber, || {
            let _scope = session.enter();
            let span = tracing::info_span!("req", model_version = "v3");
            let _entered = span.enter();
            tracing::info!(tokens = 5u64, "generated");
        });

        let logs = sink.logs.lock().unwrap();
        assert_eq!(logs.len(), 1);
        let log = &logs[0];
        assert_eq!(log.message, "generated");
        let metadata = log.metadata.as_object().unwrap();
        assert_eq!(metadata.get("request_id").unwrap().as_str(), Some("req-1"));
        assert_eq!(metadata.get("model_version").unwrap().as_str(), Some("v3"));
        assert_eq!(metadata.get("tokens").unwrap().as_u64(), Some(5));
    }

    #[test]
    fn tracing_event_ignored_without_ambient_session() {
        let sink = Arc::new(RecordingSink::default());
        let _session = session(sink.clone());
        let subscriber = tracing_subscriber::registry().with(inference_log_layer());

        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("no ambient session");
        });

        assert!(sink.logs.lock().unwrap().is_empty());
    }

    #[test]
    fn explicit_session_logging_carries_scoped_attributes() {
        let sink = Arc::new(RecordingSink::default());
        let session = session(sink.clone());

        session
            .with_attributes([("model_version", "v3")])
            .log_gauge("tokens_per_s", 12.5);

        let metrics = sink.metrics.lock().unwrap();
        assert_eq!(metrics.len(), 1);
        assert_eq!(metrics[0].name, "tokens_per_s");
        let metadata = metrics[0].metadata.as_object().unwrap();
        assert_eq!(metadata.get("request_id").unwrap().as_str(), Some("req-1"));
        assert_eq!(metadata.get("model_version").unwrap().as_str(), Some("v3"));
    }
}
