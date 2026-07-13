use serde_json::{Map, Value};
use tracing_subscriber::registry::LookupSpan;

use crate::InferenceSession;
use crate::integration::tracing::visitor::JsonVisitor;
use crate::sink::LogLevel;

/// Span fields captured for later inclusion in event metadata.
#[derive(Debug, Default)]
struct SpanFields(Map<String, Value>);

/// `tracing_subscriber` layer that forwards events to the ambient [`InferenceSession`]. Events
/// with no ambient session are ignored. Construct it via [`super::inference_log_layer`].
#[derive(Debug, Default)]
pub struct InferenceTracingLogLayer;

impl<S> tracing_subscriber::Layer<S> for InferenceTracingLogLayer
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
        let mut visitor = JsonVisitor::default();
        attrs.record(&mut visitor);
        span.extensions_mut().insert(SpanFields(visitor.fields));
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
        let mut visitor = JsonVisitor::default();
        values.record(&mut visitor);
        let mut extensions = span.extensions_mut();
        if let Some(existing) = extensions.get_mut::<SpanFields>() {
            existing.0.extend(visitor.fields);
        } else {
            extensions.insert(SpanFields(visitor.fields));
        }
    }

    fn on_event(&self, event: &tracing::Event<'_>, ctx: tracing_subscriber::layer::Context<'_, S>) {
        let Some(session) = InferenceSession::current() else {
            return;
        };

        let mut metadata = Map::new();
        if let Some(scope) = ctx.event_scope(event) {
            for span in scope.from_root() {
                if let Some(fields) = span.extensions().get::<SpanFields>() {
                    for (key, value) in fields.0.iter() {
                        metadata.insert(key.clone(), value.clone());
                    }
                }
            }
        }

        let mut visitor = JsonVisitor::default();
        event.record(&mut visitor);
        metadata.extend(visitor.fields);

        let level = map_level(event.metadata().level());
        let message = visitor.message.unwrap_or_default();
        session.record_log(level, message, Some(Value::Object(metadata)));
    }
}

fn map_level(level: &tracing::Level) -> LogLevel {
    match *level {
        tracing::Level::TRACE => LogLevel::Trace,
        tracing::Level::DEBUG => LogLevel::Debug,
        tracing::Level::INFO => LogLevel::Info,
        tracing::Level::WARN => LogLevel::Warn,
        tracing::Level::ERROR => LogLevel::Error,
    }
}
