use std::fmt;
use std::sync::Arc;

use serde_json::Value;

use crate::observer::InferenceOutputObserver;
use crate::sink::{
    InferenceSink, LogLevel, LogSample, MetricData, MetricDescriptor, MetricSample, now_ms,
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
    observer: Arc<dyn InferenceOutputObserver>,
    sink: Arc<dyn InferenceSink>,
    attrs: Arc<Vec<(String, Value)>>,
}

impl InferenceSession {
    /// Create a session. The id is seeded as a `request_id` scoped attribute on all telemetry.
    pub fn new(
        id: impl Into<InferenceId>,
        observer: Arc<dyn InferenceOutputObserver>,
        sink: Arc<dyn InferenceSink>,
    ) -> Self {
        let id = id.into();
        let attrs = vec![("request_id".to_string(), Value::String(id.to_string()))];
        Self {
            id,
            observer,
            sink,
            attrs: Arc::new(attrs),
        }
    }

    /// Borrow the session identifier.
    pub fn id(&self) -> &InferenceId {
        &self.id
    }

    /// The observer to attach to the request's output writer.
    pub fn observer(&self) -> Arc<dyn InferenceOutputObserver> {
        self.observer.clone()
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
            observer: self.observer.clone(),
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
}
