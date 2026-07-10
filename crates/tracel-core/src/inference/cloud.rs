//! Ships inference session telemetry to the backend inference-group endpoint. One long-lived
//! worker per group batches events from all its requests and flushes them over the client.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use chrono::SecondsFormat;
use crossbeam::channel::{self, Receiver, RecvTimeoutError, Sender};

use tracel_client::inference::request::{
    IngestTelemetryRequest, LogIngestionEvent, LogLevel as WireLogLevel,
    MetricData as WireMetricData, MetricDescriptorEvent, MetricIngestionEvent,
    MetricKind as WireMetricKind,
};
use tracel_client::{Client, ClientError};
use tracel_inference::observer::{InferenceOutputObserver, InferenceOutputStats};
use tracel_inference::sink::{
    InferenceSink, LogLevel, LogSample, MetricData, MetricDescriptor, MetricKind, MetricSample,
    now_ms,
};
use tracel_inference::{InferenceError, InferenceProvider, InferenceSession};

const FLUSH_INTERVAL: Duration = Duration::from_millis(250);
const MAX_BATCH: usize = 512;

/// Inference provider that ships session telemetry to the cloud backend.
pub struct CloudInferenceProvider {
    client: Client,
    namespace: String,
    project: String,
    groups: Mutex<HashMap<String, Arc<GroupTelemetryWorker>>>,
    request_counter: AtomicU64,
}

impl CloudInferenceProvider {
    pub fn new(client: Client, namespace: String, project: String) -> Self {
        Self {
            client,
            namespace,
            project,
            groups: Mutex::new(HashMap::new()),
            request_counter: AtomicU64::new(0),
        }
    }

    fn ensure_group(&self, name: &str) -> Result<Arc<GroupTelemetryWorker>, InferenceError> {
        let mut groups = self.groups.lock().unwrap();
        if let Some(worker) = groups.get(name) {
            return Ok(worker.clone());
        }

        self.ensure_group_exists(name)?;

        let worker = Arc::new(GroupTelemetryWorker::spawn(
            self.client.clone(),
            self.namespace.clone(),
            self.project.clone(),
            name.to_string(),
        ));
        groups.insert(name.to_string(), worker.clone());
        Ok(worker)
    }

    fn ensure_group_exists(&self, name: &str) -> Result<(), InferenceError> {
        match self
            .client
            .get_inference_group(&self.namespace, &self.project, name)
        {
            Ok(_) => Ok(()),
            Err(ClientError::NotFound) => {
                match self.client.create_inference_group(
                    &self.namespace,
                    &self.project,
                    name.to_string(),
                    None,
                ) {
                    Ok(_) => Ok(()),
                    // Another creator won the race.
                    Err(ClientError::ApiError { status, .. }) if status.as_u16() == 409 => Ok(()),
                    Err(err) => Err(client_error(name, err)),
                }
            }
            Err(err) => Err(client_error(name, err)),
        }
    }
}

impl InferenceProvider for CloudInferenceProvider {
    fn create_session(&self, name: &str) -> Result<InferenceSession, InferenceError> {
        let worker = self.ensure_group(name)?;
        let n = self.request_counter.fetch_add(1, Ordering::Relaxed);
        let request_id = format!("{name}/{n}");

        let sink: Arc<dyn InferenceSink> = Arc::new(ChannelSink {
            tx: worker.sender(),
        });
        let observer = Arc::new(CloudRequestObserver {
            inference_name: name.to_string(),
            request_id: request_id.clone(),
            sink: sink.clone(),
        });

        Ok(InferenceSession::new(request_id, observer, sink))
    }
}

fn client_error(group: &str, err: ClientError) -> InferenceError {
    InferenceError::with_source(format!("inference group `{group}`: {err}"), err)
}

struct ChannelSink {
    tx: Sender<TelemetryMsg>,
}

impl InferenceSink for ChannelSink {
    fn record_metric(&self, sample: MetricSample) {
        let _ = self.tx.send(TelemetryMsg::Metric(sample));
    }

    fn record_log(&self, sample: LogSample) {
        let _ = self.tx.send(TelemetryMsg::Log(sample));
    }

    fn record_descriptor(&self, descriptor: MetricDescriptor) {
        let _ = self.tx.send(TelemetryMsg::Descriptor(descriptor));
    }
}

/// Records per-request stat metrics when a request completes.
struct CloudRequestObserver {
    inference_name: String,
    request_id: String,
    sink: Arc<dyn InferenceSink>,
}

impl InferenceOutputObserver for CloudRequestObserver {
    fn on_finish(&self, stats: &InferenceOutputStats) {
        let timestamp_ms = now_ms();
        let metadata = serde_json::json!({
            "request_id": self.request_id,
            "inference_name": self.inference_name,
            "cancelled": stats.cancelled,
        });
        let duration_ms = stats.duration.as_secs_f64() * 1_000.0;

        let record = |name: &str, data: MetricData| {
            self.sink.record_metric(MetricSample {
                name: name.to_string(),
                timestamp_ms,
                metadata: metadata.clone(),
                data,
            });
        };

        record("inference_requests_total", MetricData::Counter { value: 1 });
        record(
            "inference_outputs_total",
            MetricData::Counter {
                value: stats.outputs as u64,
            },
        );
        record(
            "inference_errors_total",
            MetricData::Counter {
                value: stats.errors as u64,
            },
        );
        // Raw sample; the backend computes quantiles over the accumulated durations.
        record(
            "inference_duration_ms",
            MetricData::Distribution { value: duration_ms },
        );
    }
}

enum TelemetryMsg {
    Metric(MetricSample),
    Log(LogSample),
    Descriptor(MetricDescriptor),
    Shutdown,
}

/// Long-lived per-group worker that batches and flushes telemetry.
struct GroupTelemetryWorker {
    tx: Sender<TelemetryMsg>,
    handle: Option<JoinHandle<()>>,
}

impl GroupTelemetryWorker {
    fn spawn(client: Client, namespace: String, project: String, group: String) -> Self {
        let (tx, rx) = channel::unbounded();
        let handle = std::thread::spawn(move || {
            run_worker(client, namespace, project, group, rx);
        });
        Self {
            tx,
            handle: Some(handle),
        }
    }

    fn sender(&self) -> Sender<TelemetryMsg> {
        self.tx.clone()
    }
}

impl Drop for GroupTelemetryWorker {
    fn drop(&mut self) {
        let _ = self.tx.send(TelemetryMsg::Shutdown);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn run_worker(
    client: Client,
    namespace: String,
    project: String,
    group: String,
    rx: Receiver<TelemetryMsg>,
) {
    let mut batch = Batch::default();

    loop {
        match rx.recv_timeout(FLUSH_INTERVAL) {
            Ok(TelemetryMsg::Shutdown) => {
                // Drain anything still queued before the final flush.
                while let Ok(message) = rx.try_recv() {
                    if !matches!(message, TelemetryMsg::Shutdown) {
                        batch.push(message);
                    }
                }
                batch.flush(&client, &namespace, &project, &group);
                break;
            }
            Ok(message) => {
                batch.push(message);
                if batch.len() >= MAX_BATCH {
                    batch.flush(&client, &namespace, &project, &group);
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                batch.flush(&client, &namespace, &project, &group);
            }
            Err(RecvTimeoutError::Disconnected) => {
                batch.flush(&client, &namespace, &project, &group);
                break;
            }
        }
    }
}

#[derive(Default)]
struct Batch {
    metrics: Vec<MetricIngestionEvent>,
    descriptors: Vec<MetricDescriptorEvent>,
    logs: Vec<LogIngestionEvent>,
    seen_descriptors: HashSet<String>,
}

impl Batch {
    fn len(&self) -> usize {
        self.metrics.len() + self.descriptors.len() + self.logs.len()
    }

    fn push(&mut self, message: TelemetryMsg) {
        match message {
            TelemetryMsg::Metric(sample) => self.metrics.push(convert_metric(sample)),
            TelemetryMsg::Log(sample) => self.logs.push(convert_log(sample)),
            TelemetryMsg::Descriptor(descriptor) => {
                // Deduplicate over the worker's lifetime.
                if self.seen_descriptors.insert(descriptor.name.clone()) {
                    self.descriptors.push(convert_descriptor(descriptor));
                }
            }
            TelemetryMsg::Shutdown => {}
        }
    }

    fn flush(&mut self, client: &Client, namespace: &str, project: &str, group: &str) {
        if self.metrics.is_empty() && self.descriptors.is_empty() && self.logs.is_empty() {
            return;
        }

        let request = IngestTelemetryRequest {
            metrics: std::mem::take(&mut self.metrics),
            metric_descriptors: std::mem::take(&mut self.descriptors),
            logs: std::mem::take(&mut self.logs),
        };

        if let Err(err) = client.ingest_inference_telemetry(namespace, project, group, request) {
            tracing::warn!(
                error = %err,
                group = %group,
                "failed to ship inference telemetry batch"
            );
        }
    }
}

fn convert_metric(sample: MetricSample) -> MetricIngestionEvent {
    MetricIngestionEvent {
        name: sample.name,
        timestamp: rfc3339(sample.timestamp_ms),
        metadata: sample.metadata,
        data: match sample.data {
            MetricData::Gauge { value } => WireMetricData::Gauge { value },
            MetricData::Counter { value } => WireMetricData::Counter { value },
            MetricData::Distribution { value } => WireMetricData::Distribution { value },
        },
    }
}

fn convert_log(sample: LogSample) -> LogIngestionEvent {
    LogIngestionEvent {
        timestamp: rfc3339(sample.timestamp_ms),
        level: convert_level(sample.level),
        message: sample.message,
        metadata: sample.metadata,
    }
}

fn convert_descriptor(descriptor: MetricDescriptor) -> MetricDescriptorEvent {
    MetricDescriptorEvent {
        name: descriptor.name,
        kind: convert_kind(descriptor.kind),
        unit: descriptor.unit,
        description: descriptor.description,
    }
}

fn convert_level(level: LogLevel) -> WireLogLevel {
    match level {
        LogLevel::Trace => WireLogLevel::Trace,
        LogLevel::Debug => WireLogLevel::Debug,
        LogLevel::Info => WireLogLevel::Info,
        LogLevel::Warn => WireLogLevel::Warn,
        LogLevel::Error => WireLogLevel::Error,
    }
}

fn convert_kind(kind: MetricKind) -> WireMetricKind {
    match kind {
        MetricKind::Gauge => WireMetricKind::Gauge,
        MetricKind::Counter => WireMetricKind::Counter,
        MetricKind::Distribution => WireMetricKind::Distribution,
    }
}

fn rfc3339(timestamp_ms: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(timestamp_ms)
        .unwrap_or_else(chrono::Utc::now)
        .to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_formats_epoch_millis() {
        assert_eq!(rfc3339(1000), "1970-01-01T00:00:01.000Z");
    }

    #[test]
    fn converts_metric_sample_to_backend_contract() {
        let sample = MetricSample {
            name: "inference_duration_ms".to_string(),
            timestamp_ms: 1000,
            metadata: serde_json::json!({ "request_id": "wordtok/0" }),
            data: MetricData::Distribution { value: 5.0 },
        };

        let json = serde_json::to_value(convert_metric(sample)).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "name": "inference_duration_ms",
                "timestamp": "1970-01-01T00:00:01.000Z",
                "metadata": { "request_id": "wordtok/0" },
                "kind": "distribution",
                "value": 5.0
            })
        );
    }

    #[test]
    fn batch_dedups_descriptors_and_counts_events() {
        let descriptor = || {
            TelemetryMsg::Descriptor(MetricDescriptor {
                name: "latency".to_string(),
                kind: MetricKind::Distribution,
                unit: Some("ms".to_string()),
                description: None,
            })
        };

        let mut batch = Batch::default();
        batch.push(descriptor());
        batch.push(descriptor());
        batch.push(TelemetryMsg::Log(LogSample {
            timestamp_ms: 0,
            level: LogLevel::Info,
            message: "hi".to_string(),
            metadata: serde_json::json!({}),
        }));
        batch.push(TelemetryMsg::Metric(MetricSample {
            name: "latency".to_string(),
            timestamp_ms: 0,
            metadata: serde_json::json!({}),
            data: MetricData::Gauge { value: 1.0 },
        }));

        assert_eq!(batch.descriptors.len(), 1);
        assert_eq!(batch.logs.len(), 1);
        assert_eq!(batch.metrics.len(), 1);
        assert_eq!(batch.len(), 3);
    }
}
