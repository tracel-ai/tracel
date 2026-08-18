use std::num::NonZeroU64;

use serde_json::{Map, Value};
use tracing::field::{Field, Visit};
use tracing_subscriber::registry::LookupSpan;

use crate::{
    ActivityId, ExperimentId, ExperimentRun, LogLevel, LogRecord, context::ExperimentGlobalExt,
    integration::tracing::registry::TracingRegistry,
};

/// Field names that control routing rather than describe a log record.
const EXPERIMENT_ID_FIELD: &str = "experiment_id";
const ACTIVITY_ID_FIELD: &str = "activity_id";
const MESSAGE_FIELD: &str = "message";

/// `tracing_subscriber` layer that forwards events into experiment logs as structured records.
///
/// The layer resolves the destination and scope from the event's span context in one walk:
/// - the run is chosen from a span-bound `experiment_id` (see
///   [`super::ExperimentTracingExt::tracing_span`]), falling back to the ambient
///   [`crate::ExperimentGlobalExt`] experiment;
/// - the record is scoped to the nearest span-bound `activity_id` (see
///   [`super::ActivityTracingExt::tracing_span`]), if any.
///
/// Construct it directly or use [`super::tracing_log_layer`] for a named helper function.
#[derive(Debug, Default)]
pub struct ExperimentTracingLogLayer;

impl<S> tracing_subscriber::Layer<S> for ExperimentTracingLogLayer
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        id: &tracing::span::Id,
        ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let Some(span) = ctx.span(id) else {
            return;
        };

        span.extensions_mut()
            .insert(SpanFields::from_attributes(attrs));
    }

    fn on_record(
        &self,
        id: &tracing::span::Id,
        values: &tracing::span::Record<'_>,
        ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let Some(span) = ctx.span(id) else {
            return;
        };

        let mut extensions = span.extensions_mut();
        let updates = SpanFields::from_record(values);
        if let Some(existing) = extensions.get_mut::<SpanFields>() {
            existing.merge(updates);
        } else {
            extensions.insert(updates);
        }
    }

    fn on_event(&self, event: &tracing::Event<'_>, ctx: tracing_subscriber::layer::Context<'_, S>) {
        let metadata = event.metadata();
        if metadata.target().starts_with("wgpu") && *metadata.level() == tracing::Level::INFO {
            return;
        }

        let mut experiment_id = None;
        let mut activity_id = None;
        // Attributes inherited from enclosing spans, accumulated outermost-first so inner spans
        // override outer ones.
        let mut attributes = Map::new();
        if let Some(scope) = ctx.event_scope(event) {
            for span in scope.from_root() {
                if let Some(fields) = span.extensions().get::<SpanFields>() {
                    if fields.experiment_id.is_some() {
                        experiment_id = fields.experiment_id.clone();
                    }
                    if fields.activity_id.is_some() {
                        activity_id = fields.activity_id;
                    }
                    attributes.extend(fields.attributes.clone());
                }
            }
        }

        let handle = match experiment_id {
            Some(experiment_id) => match TracingRegistry::global().get_handle(&experiment_id) {
                Some(handle) => handle,
                None => return,
            },
            None => match ExperimentRun::current() {
                Some(handle) => handle,
                None => return,
            },
        };
        let handle = match activity_id {
            Some(activity_id) => {
                let cancel_token = handle.cancel_token();
                handle.for_activity(activity_id, cancel_token)
            }
            None => handle,
        };

        let mut visitor = JsonFieldVisitor::default();
        event.record(&mut visitor);
        let message = visitor
            .fields
            .remove(MESSAGE_FIELD)
            .map(|value| match value {
                Value::String(text) => text,
                other => other.to_string(),
            })
            .unwrap_or_default();
        // The event's own fields take precedence over inherited span scope.
        attributes.extend(visitor.fields);

        handle.log(LogRecord {
            level: log_level(metadata.level()),
            message,
            attributes,
        });
    }
}

fn log_level(level: &tracing::Level) -> LogLevel {
    match *level {
        tracing::Level::TRACE => LogLevel::Trace,
        tracing::Level::DEBUG => LogLevel::Debug,
        tracing::Level::INFO => LogLevel::Info,
        tracing::Level::WARN => LogLevel::Warn,
        tracing::Level::ERROR => LogLevel::Error,
    }
}

/// Routing identifiers and inherited attributes stored in a span's extensions.
struct SpanFields {
    experiment_id: Option<ExperimentId>,
    activity_id: Option<ActivityId>,
    attributes: Map<String, Value>,
}

impl SpanFields {
    fn merge(&mut self, other: Self) {
        if other.experiment_id.is_some() {
            self.experiment_id = other.experiment_id;
        }
        if other.activity_id.is_some() {
            self.activity_id = other.activity_id;
        }
        self.attributes.extend(other.attributes);
    }

    fn from_attributes(attrs: &tracing::span::Attributes<'_>) -> Self {
        let mut visitor = JsonFieldVisitor::default();
        attrs.record(&mut visitor);
        Self::from_fields(visitor.fields)
    }

    fn from_record(record: &tracing::span::Record<'_>) -> Self {
        let mut visitor = JsonFieldVisitor::default();
        record.record(&mut visitor);
        Self::from_fields(visitor.fields)
    }

    fn from_fields(mut fields: Map<String, Value>) -> Self {
        let experiment_id = match fields.remove(EXPERIMENT_ID_FIELD) {
            Some(Value::String(id)) => Some(ExperimentId::new(id)),
            _ => None,
        };
        let activity_id = fields
            .remove(ACTIVITY_ID_FIELD)
            .and_then(|value| value.as_u64())
            .and_then(NonZeroU64::new)
            .map(ActivityId::new);

        Self {
            experiment_id,
            activity_id,
            attributes: fields,
        }
    }
}

/// Converts tracing field values into the JSON representation used by experiment log records.
/// Interpretation of reserved fields stays with the layer that owns the routing policy.
#[derive(Default)]
struct JsonFieldVisitor {
    fields: Map<String, Value>,
}

impl JsonFieldVisitor {
    fn record(&mut self, field: &Field, value: Value) {
        self.fields.insert(field.name().to_string(), value);
    }
}

impl Visit for JsonFieldVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.record(field, Value::String(value.to_string()));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.record(field, Value::from(value));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.record(field, Value::from(value));
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.record(field, Value::from(value));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.record(field, Value::from(value));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.record(field, Value::String(format!("{value:?}")));
    }
}
