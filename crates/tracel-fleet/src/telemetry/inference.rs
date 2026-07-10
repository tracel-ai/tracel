use tracel_inference::sink::{InferenceSink, LogLevel, LogSample, MetricData, MetricSample};

/// Fleet metadata attached to each inference request as scoped session attributes.
#[derive(Debug, Clone)]
pub struct InferenceMetadata {
    pub fleet_key: String,
    pub inference_name: String,
    pub model_name: String,
    pub model_version: String,
}

impl InferenceMetadata {
    pub fn new(
        fleet_key: impl Into<String>,
        inference_name: impl Into<String>,
        model_name: impl Into<String>,
        model_version: impl Into<String>,
    ) -> Self {
        Self {
            fleet_key: fleet_key.into(),
            inference_name: inference_name.into(),
            model_name: model_name.into(),
            model_version: model_version.into(),
        }
    }

    /// The metadata as scoped-attribute pairs for `InferenceSession::with_attributes`.
    pub fn attributes(&self) -> [(&'static str, String); 4] {
        [
            ("fleet_key", self.fleet_key.clone()),
            ("inference_name", self.inference_name.clone()),
            ("model_name", self.model_name.clone()),
            ("model_version", self.model_version.clone()),
        ]
    }
}

impl Default for InferenceMetadata {
    fn default() -> Self {
        Self::new("unknown", "unknown", "unknown", "unknown")
    }
}

/// [`InferenceSink`] that routes inference telemetry to the process-global `metrics` recorder and to
/// `tracing`.
///
/// Per-request stats and any metrics the inference records become `tracel_fleet_`-prefixed metrics,
/// with the session's scoped attributes as labels (excluding the high-cardinality `request_id`).
/// Logs are forwarded to `tracing` at their level.
pub struct MetricsSink;

impl InferenceSink for MetricsSink {
    fn record_metric(&self, sample: MetricSample) {
        let name = format!("tracel_fleet_{}", sample.name);
        let labels = metric_labels(&sample.metadata);
        match sample.data {
            MetricData::Counter { value } => ::metrics::counter!(name, labels).increment(value),
            MetricData::Gauge { value } => ::metrics::gauge!(name, labels).set(value),
            MetricData::Distribution { value } => ::metrics::histogram!(name, labels).record(value),
        }
    }

    fn record_log(&self, sample: LogSample) {
        let metadata = sample.metadata;
        match sample.level {
            LogLevel::Error => tracing::error!(%metadata, "{}", sample.message),
            LogLevel::Warn => tracing::warn!(%metadata, "{}", sample.message),
            LogLevel::Info => tracing::info!(%metadata, "{}", sample.message),
            LogLevel::Debug => tracing::debug!(%metadata, "{}", sample.message),
            LogLevel::Trace => tracing::trace!(%metadata, "{}", sample.message),
        }
    }
}

/// Session attributes become metric labels, excluding the high-cardinality `request_id`.
fn metric_labels(metadata: &serde_json::Value) -> Vec<::metrics::Label> {
    let Some(object) = metadata.as_object() else {
        return Vec::new();
    };
    object
        .iter()
        .filter(|(key, _)| key.as_str() != "request_id")
        .map(|(key, value)| {
            let value = match value {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            ::metrics::Label::new(key.clone(), value)
        })
        .collect()
}
