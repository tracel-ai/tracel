use std::fmt;
use std::sync::Arc;

use serde_json::Value;

use crate::inference::Inference;
use crate::input::InferenceInput;
use crate::observer::{InferenceOutputObserver, InferenceOutputStats};
use crate::output::{InferenceOutput, OutputWriter};
use crate::sink::{
    InferenceSink, LogLevel, LogSample, MetricData, MetricDescriptor, MetricSample, NoopSink,
    now_ms,
};

/// Opaque identifier for a single inference request/session.
///
/// The identifier format is backend-specific and stable only for the backend that created it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InferenceId(String);

impl InferenceId {
    /// Create an identifier from a backend-specific string value.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Borrow the backend-specific identifier value.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for InferenceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for InferenceId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for InferenceId {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

/// The telemetry surface for one inference request.
///
/// Scoped attributes ride along as the `metadata` of everything recorded through the session.
#[derive(Clone)]
pub struct InferenceSession {
    id: InferenceId,
    sink: Arc<dyn InferenceSink>,
    attrs: Arc<Vec<(String, Value)>>,
}

impl InferenceSession {
    /// Create a session recording into `sink`. The id is seeded as a `request_id` scoped attribute
    /// on all telemetry. Per-request stats and any metrics/logs the inference records flow to the
    /// sink, so a backend only needs to implement [`InferenceSink`](crate::sink::InferenceSink).
    pub fn new(id: impl Into<InferenceId>, sink: Arc<dyn InferenceSink>) -> Self {
        let id = id.into();
        let attrs = vec![("request_id".to_string(), Value::String(id.to_string()))];
        Self {
            id,
            sink,
            attrs: Arc::new(attrs),
        }
    }

    /// Borrow the session identifier.
    pub fn id(&self) -> &InferenceId {
        &self.id
    }

    /// The sink this session records into.
    pub fn sink(&self) -> Arc<dyn InferenceSink> {
        self.sink.clone()
    }

    /// Derive a child handle that adds scoped attributes to every metric and log it records.
    ///
    /// Later keys override earlier ones, so attributes added here win over the base `request_id`.
    pub fn with_attributes<K, V>(&self, attributes: impl IntoIterator<Item = (K, V)>) -> Self
    where
        K: Into<String>,
        V: Into<Value>,
    {
        let mut attrs = (*self.attrs).clone();
        for (key, value) in attributes {
            attrs.push((key.into(), value.into()));
        }
        Self {
            id: self.id.clone(),
            sink: self.sink.clone(),
            attrs: Arc::new(attrs),
        }
    }

    /// Record a metric with this session's scoped attributes as its metadata.
    pub fn log_metric(&self, name: impl Into<String>, data: MetricData) {
        self.sink.record_metric(MetricSample {
            name: name.into(),
            timestamp_ms: now_ms(),
            metadata: self.metadata(),
            data,
        });
    }

    /// Record a gauge sample.
    pub fn log_gauge(&self, name: impl Into<String>, value: f64) {
        self.log_metric(name, MetricData::Gauge { value });
    }

    /// Record a counter increment (a delta).
    pub fn log_counter(&self, name: impl Into<String>, value: u64) {
        self.log_metric(name, MetricData::Counter { value });
    }

    /// Record a raw observation (e.g. a latency sample); quantiles are computed server-side.
    pub fn log_distribution(&self, name: impl Into<String>, value: f64) {
        self.log_metric(name, MetricData::Distribution { value });
    }

    /// Declare a descriptor (unit, description) for a metric name.
    pub fn describe_metric(&self, descriptor: MetricDescriptor) {
        self.sink.record_descriptor(descriptor);
    }

    /// Record a log line with this session's scoped attributes as its metadata.
    pub fn log(&self, level: LogLevel, message: impl Into<String>) {
        self.record_log(level, message.into(), None);
    }

    /// Record a log line, merging any extra per-event attributes over the session's.
    pub(crate) fn record_log(&self, level: LogLevel, message: String, extra: Option<Value>) {
        self.sink.record_log(LogSample {
            timestamp_ms: now_ms(),
            level,
            message,
            metadata: self.metadata_with(extra),
        });
    }

    fn metadata(&self) -> Value {
        let mut map = serde_json::Map::with_capacity(self.attrs.len());
        for (key, value) in self.attrs.iter() {
            map.insert(key.clone(), value.clone());
        }
        Value::Object(map)
    }

    fn metadata_with(&self, extra: Option<Value>) -> Value {
        let mut base = self.metadata();
        if let (Value::Object(base_map), Some(Value::Object(extra_map))) = (&mut base, extra) {
            for (key, value) in extra_map {
                base_map.insert(key, value);
            }
        }
        base
    }

    /// The ambient session for the current thread, if any.
    ///
    /// Available inside `Inference::infer` when the job installed a session for the request.
    pub fn current() -> Option<InferenceSession> {
        crate::context::current_session()
    }

    /// Install this session as the ambient session for the current thread until the guard drops.
    pub fn enter(&self) -> crate::context::SessionGuard {
        crate::context::enter(self.clone())
    }

    /// A session backed by a [`NoopSink`](crate::sink::NoopSink): everything recorded is discarded.
    ///
    /// For driving an inference without a telemetry backend (tests, offline scratch). Build a real
    /// session with [`new`](Self::new) to ship.
    pub fn noop() -> Self {
        Self::new("noop", Arc::new(NoopSink))
    }

    /// Drive `inference` to completion on the calling thread under this session.
    ///
    /// Installs this session as the ambient session for the thread, attaches its observer to the
    /// output, feeds `input`, and passes `self` to [`Inference::infer`]. This is the provider-free
    /// driver: pair it with [`new`](Self::new) or [`noop`](Self::noop) to run any [`Inference`]
    /// without an [`InferenceProvider`](crate::InferenceProvider) or
    /// [`InferenceJob`](crate::InferenceJob).
    pub fn run<Inf, It, W>(&self, inference: &Inf, input: It, output: W)
    where
        Inf: Inference + ?Sized,
        It: IntoIterator<Item = Inf::Input>,
        It::IntoIter: Send + 'static,
        W: OutputWriter<Inf::Output> + 'static,
    {
        let _scope = self.enter();
        let input = InferenceInput::from_items(input.into_iter());
        let output =
            InferenceOutput::from_writer(output).with_observer(Arc::new(SessionStatsObserver {
                session: self.clone(),
            }));
        inference.infer(self, input, output);
    }
}

/// Records per-request output statistics to the session's sink when the request completes, tagged
/// with the session's scoped attributes plus `cancelled`. Installed by [`InferenceSession::run`] so
/// every backend gets uniform request stats through its [`InferenceSink`](crate::sink::InferenceSink),
/// with no per-backend observer.
struct SessionStatsObserver {
    session: InferenceSession,
}

impl InferenceOutputObserver for SessionStatsObserver {
    fn on_finish(&self, stats: &InferenceOutputStats) {
        let session = self
            .session
            .with_attributes([("cancelled", stats.cancelled)]);
        session.log_counter("inference_requests_total", 1);
        session.log_counter("inference_outputs_total", stats.outputs as u64);
        session.log_counter("inference_errors_total", stats.errors as u64);
        session.log_distribution(
            "inference_duration_ms",
            stats.duration.as_secs_f64() * 1_000.0,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Echo;
    impl Inference for Echo {
        type Input = i32;
        type Output = i32;
        fn infer(
            &self,
            _session: &InferenceSession,
            input: InferenceInput<i32>,
            output: InferenceOutput<i32>,
        ) {
            // The session is installed as ambient for this thread while running.
            assert!(InferenceSession::current().is_some());
            for item in input {
                let _ = output.write(item);
            }
        }
    }

    struct VecWriter(Arc<Mutex<Vec<i32>>>);
    impl OutputWriter<i32> for VecWriter {
        fn write(&self, output: i32) -> Result<(), crate::OutputWriterError> {
            self.0.lock().unwrap().push(output);
            Ok(())
        }
        fn error(
            &self,
            _error: Box<dyn std::error::Error + Send + Sync>,
        ) -> Result<(), crate::OutputWriterError> {
            Ok(())
        }
        fn finish(&self, _duration: std::time::Duration) {}
    }

    #[derive(Default)]
    struct RecordingSink {
        metrics: Mutex<Vec<MetricSample>>,
    }
    impl InferenceSink for RecordingSink {
        fn record_metric(&self, sample: MetricSample) {
            self.metrics.lock().unwrap().push(sample);
        }
        fn record_log(&self, _sample: LogSample) {}
    }

    #[test]
    fn run_drives_an_inference_without_a_provider_or_job() {
        let collected = Arc::new(Mutex::new(Vec::new()));
        InferenceSession::noop().run(&Echo, vec![1, 2, 3], VecWriter(collected.clone()));
        assert_eq!(*collected.lock().unwrap(), vec![1, 2, 3]);
        // Ambient session is cleared once `run` returns.
        assert!(InferenceSession::current().is_none());
    }

    #[test]
    fn run_records_request_stats_to_the_sink() {
        let sink = Arc::new(RecordingSink::default());
        let discard = Arc::new(Mutex::new(Vec::new()));
        InferenceSession::new("req-42", sink.clone()).run(&Echo, vec![1, 2, 3], VecWriter(discard));

        let metrics = sink.metrics.lock().unwrap();
        let names: Vec<&str> = metrics.iter().map(|m| m.name.as_str()).collect();
        for expected in [
            "inference_requests_total",
            "inference_outputs_total",
            "inference_errors_total",
            "inference_duration_ms",
        ] {
            assert!(names.contains(&expected), "missing stat metric {expected}");
        }

        let outputs = metrics
            .iter()
            .find(|m| m.name == "inference_outputs_total")
            .unwrap();
        assert!(matches!(&outputs.data, MetricData::Counter { value: 3 }));

        // Stats carry the session's scoped attributes plus `cancelled`.
        let meta = outputs.metadata.as_object().unwrap();
        assert_eq!(meta.get("request_id").unwrap().as_str(), Some("req-42"));
        assert_eq!(meta.get("cancelled").unwrap().as_bool(), Some(false));
    }
}
