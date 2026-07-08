use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;

use axum::{
    body::Body,
    http::StatusCode,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;

use tracel_experiment::ExperimentJob;
use tracel_inference::InferenceJob;

/// Maximum request body size buffered by non-streaming routes (10 MiB).
pub(crate) const MAX_BODY_BYTES: usize = 10 * 1024 * 1024;

/// A response produced asynchronously by a [`ServerRoute`].
pub type RouteFuture = Pin<Box<dyn Future<Output = Response> + Send>>;

/// A capability served over HTTP.
///
/// This is the server's local trait: capabilities plug in via [`IntoServerRoute`], and each decides
/// how to consume the request body and shape its response (experiments respond fire-and-forget;
/// inference consumes the body as a stream and streams SSE back). Implement it directly only for a
/// bespoke route.
pub trait ServerRoute: Send + Sync {
    /// The name used to select this route (`POST /{name}`).
    fn name(&self) -> &str;
    /// Handle a request body and produce a response.
    fn handle(&self, body: Body) -> RouteFuture;
}

/// Turns a capability job into a [`ServerRoute`].
///
/// Implemented for `ExperimentJob` (fire-and-forget) and `InferenceJob` (streaming SSE), so
/// `Server::register(job)` works uniformly for either. A new capability becomes servable by
/// implementing this for its job.
pub trait IntoServerRoute {
    fn into_server_route(self) -> Box<dyn ServerRoute>;
}

impl<I, O> IntoServerRoute for ExperimentJob<I, O>
where
    I: DeserializeOwned + Send + 'static,
    O: 'static,
{
    fn into_server_route(self) -> Box<dyn ServerRoute> {
        Box::new(ExperimentRoute::new(self))
    }
}

impl<I, O> IntoServerRoute for InferenceJob<I, O>
where
    I: DeserializeOwned + Send + 'static,
    O: Serialize + Send + Sync + 'static,
{
    fn into_server_route(self) -> Box<dyn ServerRoute> {
        Box::new(InferenceRoute::new(self))
    }
}

/// Serves an [`ExperimentJob`] fire-and-forget: buffer and parse the JSON body, start the job in the
/// background, and respond immediately.
pub(crate) struct ExperimentRoute<I, O> {
    job: ExperimentJob<I, O>,
    default: Option<Value>,
}

impl<I, O> ExperimentRoute<I, O> {
    pub(crate) fn new(job: ExperimentJob<I, O>) -> Self {
        Self { job, default: None }
    }

    pub(crate) fn with_default(job: ExperimentJob<I, O>, default: I) -> Self
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
}

impl<I, O> ServerRoute for ExperimentRoute<I, O>
where
    I: DeserializeOwned + Send + 'static,
    O: 'static,
{
    fn name(&self) -> &str {
        self.job.name()
    }

    fn handle(&self, body: Body) -> RouteFuture {
        let job = self.job.clone();
        let name = self.job.name().to_string();
        let default = self.default.clone();
        Box::pin(async move {
            let bytes = match axum::body::to_bytes(body, MAX_BODY_BYTES).await {
                Ok(bytes) => bytes,
                Err(e) => {
                    return (StatusCode::BAD_REQUEST, format!("failed to read body: {e}"))
                        .into_response();
                }
            };
            let input = match parse_with_default::<I>(&default, &bytes) {
                Ok(input) => input,
                Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
            };

            let handle = tokio::task::spawn_blocking(move || job.run(input).map(|_| ()));
            tokio::spawn(async move {
                match handle.await {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => tracing::error!("experiment '{name}' failed: {e}"),
                    Err(e) => tracing::error!("experiment '{name}' panicked: {e}"),
                }
            });

            (StatusCode::OK, "experiment has started running").into_response()
        })
    }
}

fn parse_with_default<I>(default: &Option<Value>, body: &[u8]) -> Result<I, serde_json::Error>
where
    I: DeserializeOwned,
{
    match default {
        Some(default) => {
            if body.trim_ascii().is_empty() {
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

/// Serves an [`InferenceJob`] with a streaming request and a Server-Sent Events response.
///
/// Input messages (NDJSON, one JSON object per line — a single JSON body is one message) are framed
/// off the request body *as it arrives* and fed into the running inference, whose outputs stream
/// back as SSE `data:` frames, terminated by a `done` event. Input and output stream concurrently.
pub(crate) struct InferenceRoute<I, O> {
    job: InferenceJob<I, O>,
}

impl<I, O> InferenceRoute<I, O> {
    pub(crate) fn new(job: InferenceJob<I, O>) -> Self {
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

    fn handle(&self, body: Body) -> RouteFuture {
        let job = self.job.clone();
        Box::pin(async move {
            // Inputs are pushed here by the body-reading task and pulled by the inference worker.
            let (in_tx, in_rx) = std::sync::mpsc::channel::<I>();
            // Outputs are pushed here by the inference and drained by the SSE response.
            let (sse_tx, sse_rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(64);

            // Start the inference now; it blocks pulling inputs from `in_rx` as they arrive.
            let stream = match job.stream(in_rx) {
                Ok(stream) => stream,
                Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
            };

            // Output task: serialize each output to an SSE frame.
            let out_tx = sse_tx.clone();
            tokio::task::spawn_blocking(move || {
                for item in stream {
                    let event = match item {
                        Ok(output) => match serde_json::to_string(&output) {
                            Ok(data) => Event::default().data(data),
                            Err(e) => Event::default().event("error").data(e.to_string()),
                        },
                        Err(e) => Event::default().event("error").data(e.to_string()),
                    };
                    if out_tx.blocking_send(Ok(event)).is_err() {
                        return; // client disconnected
                    }
                }
                let _ = out_tx.blocking_send(Ok(Event::default().event("done").data("")));
            });

            // Input task: frame the request body into NDJSON messages and feed them in as they land.
            tokio::spawn(async move {
                let mut data = body.into_data_stream();
                let mut buf: Vec<u8> = Vec::new();
                while let Some(chunk) = data.next().await {
                    let chunk = match chunk {
                        Ok(chunk) => chunk,
                        Err(_) => break, // client disconnected mid-body
                    };
                    buf.extend_from_slice(&chunk);
                    while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                        let mut line: Vec<u8> = buf.drain(..=pos).collect();
                        line.pop(); // drop the '\n'
                        if !feed_line::<I>(&line, &in_tx, &sse_tx).await {
                            return;
                        }
                    }
                }
                // Flush any trailing message not terminated by a newline.
                let _ = feed_line::<I>(&buf, &in_tx, &sse_tx).await;
            });

            Sse::new(ReceiverStream::new(sse_rx))
                .keep_alive(KeepAlive::default())
                .into_response()
        })
    }
}

/// Deserialize one framed line and send it to the inference. Returns `false` (stop feeding) on a
/// decode error or once the inference has stopped consuming input. Empty lines are skipped.
async fn feed_line<I>(
    line: &[u8],
    in_tx: &std::sync::mpsc::Sender<I>,
    sse_tx: &tokio::sync::mpsc::Sender<Result<Event, Infallible>>,
) -> bool
where
    I: DeserializeOwned,
{
    let trimmed = line.trim_ascii();
    if trimmed.is_empty() {
        return true;
    }
    match serde_json::from_slice::<I>(trimmed) {
        Ok(input) => in_tx.send(input).is_ok(),
        Err(e) => {
            let _ = sse_tx
                .send(Ok(Event::default().event("error").data(e.to_string())))
                .await;
            false
        }
    }
}
