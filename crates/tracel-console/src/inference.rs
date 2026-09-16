//! Ships inference session telemetry to the console's inference-group endpoint. One long-lived
//! actor per group batches events from all its requests and flushes them over the client.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_channel::{Receiver, Sender};
use chrono::SecondsFormat;
use futures::future::{Either, select};
use futures_timer::Delay;
use tracel_client::ClientError;
use tracel_client::console::inference::request::{
    IngestTelemetryRequest, LogIngestionEvent, LogLevel as WireLogLevel,
    MetricData as WireMetricData, MetricDescriptorEvent, MetricIngestionEvent,
    MetricKind as WireMetricKind,
};
use tracel_inference::sink::{
    InferenceSink, LogLevel, LogSample, MetricData, MetricDescriptor, MetricKind, MetricSample,
};
use tracel_inference::{InferenceError, InferenceProvider, InferenceSession};
use tracel_task::Job;

use crate::ConsoleError;
use crate::console::ProjectScope;

const FLUSH_INTERVAL: Duration = Duration::from_millis(250);
const MAX_BATCH: usize = 512;

/// Inference provider that ships session telemetry to the console.
pub struct ConsoleInferenceProvider {
    scope: Arc<ProjectScope>,
    groups: Arc<Mutex<HashMap<String, Arc<GroupTelemetryWorker>>>>,
    request_counter: Arc<AtomicU64>,
}

impl ConsoleInferenceProvider {
    pub fn new(scope: Arc<ProjectScope>) -> Self {
        Self {
            scope,
            groups: Arc::new(Mutex::new(HashMap::new())),
            request_counter: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl InferenceProvider for ConsoleInferenceProvider {
    fn create_session(&self, name: String) -> Job<InferenceSession, InferenceError> {
        let scope = Arc::clone(&self.scope);
        let groups = Arc::clone(&self.groups);
        let counter = Arc::clone(&self.request_counter);
        self.scope.console.attach(async move {
            let worker = ensure_group(&scope, &groups, &name).await?;
            let n = counter.fetch_add(1, Ordering::Relaxed);
            let request_id = format!("{name}/{n}");

            let sink: Arc<dyn InferenceSink> = Arc::new(ChannelSink {
                tx: worker.sender(),
            });

            Ok(InferenceSession::new(request_id, sink).with_attributes([("inference_name", name)]))
        })
    }
}

/// The group's worker, started on first use once the console knows the group.
async fn ensure_group(
    scope: &Arc<ProjectScope>,
    groups: &Mutex<HashMap<String, Arc<GroupTelemetryWorker>>>,
    name: &str,
) -> Result<Arc<GroupTelemetryWorker>, InferenceError> {
    if let Some(worker) = groups.lock().unwrap().get(name) {
        return Ok(Arc::clone(worker));
    }

    ensure_group_exists(scope, name).await?;

    let mut groups = groups.lock().unwrap();
    Ok(Arc::clone(groups.entry(name.to_string()).or_insert_with(
        || {
            Arc::new(GroupTelemetryWorker::start(
                Arc::clone(scope),
                name.to_string(),
            ))
        },
    )))
}

async fn ensure_group_exists(scope: &ProjectScope, name: &str) -> Result<(), InferenceError> {
    let console = &scope.console;
    let found = console
        .client
        .get_inference_group(&scope.owner, &scope.project, name)
        .await;
    match found {
        Ok(_) => Ok(()),
        Err(err) if err.is_not_found() => {
            let created = console
                .client
                .create_inference_group(&scope.owner, &scope.project, name.to_string(), None)
                .await;
            match created {
                Ok(_) => Ok(()),
                // Another creator won the race.
                Err(ClientError::ApiError { status, .. }) if status.as_u16() == 409 => Ok(()),
                Err(err) => Err(client_error(name, err)),
            }
        }
        Err(err) => Err(client_error(name, err)),
    }
}

fn client_error(group: &str, error: ClientError) -> InferenceError {
    let message = format!("inference group `{group}`: {error}");
    InferenceError::with_source(message, ConsoleError::from(error))
}

/// Queues telemetry for the group's worker without waiting; the channel is unbounded.
struct ChannelSink {
    tx: Sender<TelemetryMsg>,
}

impl InferenceSink for ChannelSink {
    fn record_metric(&self, sample: MetricSample) {
        let _ = self.tx.try_send(TelemetryMsg::Metric(sample));
    }

    fn record_log(&self, sample: LogSample) {
        let _ = self.tx.try_send(TelemetryMsg::Log(sample));
    }

    fn record_descriptor(&self, descriptor: MetricDescriptor) {
        let _ = self.tx.try_send(TelemetryMsg::Descriptor(descriptor));
    }
}

enum TelemetryMsg {
    Metric(MetricSample),
    Log(LogSample),
    Descriptor(MetricDescriptor),
    Shutdown,
}

/// The mailbox of a long-lived per-group actor that batches and flushes telemetry.
///
/// The loop runs on the connection's runtime; dropping the handle asks it to flush and stop.
struct GroupTelemetryWorker {
    tx: Sender<TelemetryMsg>,
}

impl GroupTelemetryWorker {
    fn start(scope: Arc<ProjectScope>, group: String) -> Self {
        let (tx, rx) = async_channel::unbounded();
        scope
            .console
            .runtime
            .spawn(run_worker(Arc::clone(&scope), group, rx));
        Self { tx }
    }

    fn sender(&self) -> Sender<TelemetryMsg> {
        self.tx.clone()
    }
}

impl Drop for GroupTelemetryWorker {
    fn drop(&mut self) {
        let _ = self.tx.try_send(TelemetryMsg::Shutdown);
    }
}

async fn run_worker(scope: Arc<ProjectScope>, group: String, rx: Receiver<TelemetryMsg>) {
    let mut batch = Batch::default();

    loop {
        match select(std::pin::pin!(rx.recv()), Delay::new(FLUSH_INTERVAL)).await {
            Either::Left((Ok(TelemetryMsg::Shutdown), _)) => {
                // Drain anything still queued before the final flush.
                while let Ok(message) = rx.try_recv() {
                    if !matches!(message, TelemetryMsg::Shutdown) {
                        batch.push(message);
                    }
                }
                batch.flush(&scope, &group).await;
                break;
            }
            Either::Left((Ok(message), _)) => {
                batch.push(message);
                if batch.len() >= MAX_BATCH {
                    batch.flush(&scope, &group).await;
                }
            }
            Either::Right(((), _)) => {
                batch.flush(&scope, &group).await;
            }
            Either::Left((Err(_), _)) => {
                batch.flush(&scope, &group).await;
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

    async fn flush(&mut self, scope: &ProjectScope, group: &str) {
        if self.metrics.is_empty() && self.descriptors.is_empty() && self.logs.is_empty() {
            return;
        }

        let request = IngestTelemetryRequest {
            metrics: std::mem::take(&mut self.metrics),
            metric_descriptors: std::mem::take(&mut self.descriptors),
            logs: std::mem::take(&mut self.logs),
        };

        let shipped = scope
            .console
            .client
            .ingest_inference_telemetry(&scope.owner, &scope.project, group, request)
            .await;
        if let Err(err) = shipped {
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
