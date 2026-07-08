use serde_json::{Map, Value};
use tracing::field::{Field, Visit};
use tracing_subscriber::registry::LookupSpan;

use crate::{
    ExperimentRun, LogLevel, LogRecord,
    context::ExperimentGlobalExt,
    integration::tracing::{registry::TracingRegistry, visitor::SpanFields},
};

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

        let (experiment_id, activity_id) = if let Some(scope) = ctx.event_scope(event) {
            let mut experiment_id = None;
            let mut activity_id = None;
            for span in scope.from_root() {
                if let Some(fields) = span.extensions().get::<SpanFields>() {
                    if fields.experiment_id.is_some() {
                        experiment_id = fields.experiment_id.clone();
                    }
                    if fields.activity_id.is_some() {
                        activity_id = fields.activity_id;
                    }
                }
            }
            (experiment_id, activity_id)
        } else {
            (None, None)
        };

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

        let mut visitor = LogFieldVisitor::default();
        event.record(&mut visitor);

        let _ = handle.log(LogRecord {
            level: log_level(metadata.level()),
            message: visitor.message.unwrap_or_default(),
            attributes: visitor.attributes,
            activity_id,
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

/// Splits a tracing event's `message` field from its other fields, which become attributes.
#[derive(Default)]
struct LogFieldVisitor {
    message: Option<String>,
    attributes: Map<String, Value>,
}

impl LogFieldVisitor {
    fn set(&mut self, field: &Field, value: Value) {
        if field.name() == "message" {
            self.message = Some(match value {
                Value::String(text) => text,
                other => other.to_string(),
            });
        } else {
            self.attributes.insert(field.name().to_string(), value);
        }
    }
}

impl Visit for LogFieldVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.set(field, Value::String(value.to_string()));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.set(field, Value::from(value));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.set(field, Value::from(value));
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.set(field, Value::from(value));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.set(field, Value::from(value));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.set(field, Value::String(format!("{value:?}")));
    }
}
