use std::convert::Infallible;

use axum::{
    body::Bytes,
    http::StatusCode,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use tokio_stream::wrappers::ReceiverStream;

use tracel_experiment::ExperimentJob;
use tracel_inference::InferenceJob;

/// A capability served over HTTP.
///
/// This is the server's local trait: capabilities plug into the server by providing an adapter that
/// implements it (see [`ExperimentRoute`] and [`InferenceRoute`]). Each adapter reads the request
/// body and decides its own response shape.
pub trait ServerRoute: Send + Sync {
    /// The name used to select this route (`POST /{name}`).
    fn name(&self) -> &str;
    /// Handle a request body and produce a response. Called from within the tokio runtime.
    fn handle(&self, body: Bytes) -> Response;
}

/// Serves an [`ExperimentJob`] fire-and-forget: parse the JSON body, start the job in the
/// background, and respond immediately.
pub struct ExperimentRoute<I, O> {
    job: ExperimentJob<I, O>,
    default: Option<Value>,
}

impl<I, O> ExperimentRoute<I, O> {
    pub fn new(job: ExperimentJob<I, O>) -> Self {
        Self { job, default: None }
    }

    pub fn with_default(job: ExperimentJob<I, O>, default: I) -> Self
    where
        I: Serialize,
    {
        let default =
            serde_json::to_value(default).expect("default config must be serializable to JSON");
        Self {
            job,
            default: Some(default),
        }
    }

    fn parse_input(&self, body: &Bytes) -> Result<I, serde_json::Error>
    where
        I: DeserializeOwned,
    {
        match &self.default {
            Some(default) => {
                if body.iter().all(u8::is_ascii_whitespace) {
                    return serde_json::from_value(default.clone());
                }
                let overrides: Value = serde_json::from_slice(body)?;
                let mut merged = default.clone();
                json_patch::merge(&mut merged, &overrides);
                serde_json::from_value(merged)
            }
            None => serde_json::from_slice(body),
        }
    }
}

impl<I, O> ServerRoute for ExperimentRoute<I, O>
where
    I: DeserializeOwned + Send + 'static,
    O: 'static,
{
    fn name(&self) -> &str {
        self.job.name()
    }

    fn handle(&self, body: Bytes) -> Response {
        let input = match self.parse_input(&body) {
            Ok(input) => input,
            Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
        };

        let job = self.job.clone();
        let name = self.job.name().to_string();
        let handle = tokio::task::spawn_blocking(move || job.run(input).map(|_| ()));
        tokio::spawn(async move {
            match handle.await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => tracing::error!("experiment '{name}' failed: {e}"),
                Err(e) => tracing::error!("experiment '{name}' panicked: {e}"),
            }
        });

        (
            StatusCode::OK,
            format!("experiment '{}' has started running", self.job.name()),
        )
            .into_response()
    }
}

/// Serves an [`InferenceJob`] over Server-Sent Events: frame the request body into inputs, run the
/// inference, and stream each output as an SSE `data:` frame, terminated by a `done` event.
pub struct InferenceRoute<I, O> {
    job: InferenceJob<I, O>,
}

impl<I, O> InferenceRoute<I, O> {
    pub fn new(job: InferenceJob<I, O>) -> Self {
        Self { job }
    }
}

impl<I, O> ServerRoute for InferenceRoute<I, O>
where
    I: DeserializeOwned + Send + 'static,
    O: Serialize + Send + Sync + 'static,
{
    fn name(&self) -> &str {
        self.job.name()
    }

    fn handle(&self, body: Bytes) -> Response {
        let inputs: Result<Vec<I>, _> = frame_ndjson(&body)
            .into_iter()
            .map(|line| serde_json::from_slice::<I>(line))
            .collect();
        let inputs = match inputs {
            Ok(inputs) => inputs,
            Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
        };

        let stream = match self.job.stream(inputs) {
            Ok(stream) => stream,
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        };

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(64);
        tokio::task::spawn_blocking(move || {
            for item in stream {
                let event = match item {
                    Ok(output) => match serde_json::to_string(&output) {
                        Ok(data) => Event::default().data(data),
                        Err(e) => Event::default().event("error").data(e.to_string()),
                    },
                    Err(e) => Event::default().event("error").data(e.to_string()),
                };
                if tx.blocking_send(Ok(event)).is_err() {
                    return; // client disconnected
                }
            }
            let _ = tx.blocking_send(Ok(Event::default().event("done").data("")));
        });

        Sse::new(ReceiverStream::new(rx))
            .keep_alive(KeepAlive::default())
            .into_response()
    }
}

/// Split a request body into discrete JSON messages, one per non-empty line. A single JSON body
/// with no newlines is treated as one message.
fn frame_ndjson(bytes: &[u8]) -> Vec<&[u8]> {
    bytes
        .split(|&b| b == b'\n')
        .map(<[u8]>::trim_ascii)
        .filter(|line| !line.is_empty())
        .collect()
}
