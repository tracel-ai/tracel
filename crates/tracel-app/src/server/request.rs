//! How the server runs a job from a request: an experiment from one JSON document, an inference
//! from a stream of them.

use std::convert::Infallible;
use std::sync::mpsc;

use axum::{
    body::Body,
    http::StatusCode,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use serde_json::Value;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;
use tracel_inference::{OutputWriter, OutputWriterError};

use crate::{BoxError, DiscardOutput, Job, JobInput};

pub const MAX_BODY_BYTES: usize = 10 * 1024 * 1024 * 1024;

type Events = tokio::sync::mpsc::Sender<Result<Event, Infallible>>;

/// Starts an experiment with the request body, one JSON document (an empty body is no input),
/// and responds once it has started; the experiment runs in the background.
pub async fn start_experiment(job: &dyn Job, body: Body) -> Response {
    let body = match axum::body::to_bytes(body, MAX_BODY_BYTES).await {
        Ok(body) => body,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, format!("failed to read body: {e}")).into_response();
        }
    };
    let body = body.trim_ascii();
    let input = if body.is_empty() {
        Value::Null
    } else {
        match serde_json::from_slice(body) {
            Ok(input) => input,
            Err(e) => {
                return (StatusCode::BAD_REQUEST, format!("invalid JSON: {e}")).into_response();
            }
        }
    };
    let prepared = match job.prepare(JobInput::Document(input)) {
        Ok(prepared) => prepared,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };

    let name = job.definition().name.clone();
    let handle = tokio::task::spawn_blocking(move || prepared.run(DiscardOutput));
    tokio::spawn(async move {
        match handle.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::error!("experiment '{name}' failed: {e}"),
            Err(e) => tracing::error!("experiment '{name}' panicked: {e}"),
        }
    });

    (StatusCode::OK, "experiment has started running").into_response()
}

/// Runs an inference over a streaming request, answering with Server-Sent Events.
///
/// The request body is NDJSON, one JSON document per line (a single document is one input).
/// Inputs are fed to the inference as they arrive while its outputs stream back as SSE `data:`
/// frames, terminated by a `done` event. A line that is not JSON, or an input that does not decode,
/// is answered with an `error` event and ends the input.
pub fn stream_inference(job: &dyn Job, body: Body) -> Response {
    let (inputs, received) = mpsc::channel::<Value>();
    let prepared = match job.prepare(JobInput::Stream(Box::new(received.into_iter()))) {
        Ok(prepared) => prepared,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };
    let (events, sent) = tokio::sync::mpsc::channel(64);

    // The inference runs on the server's blocking pool, pulling inputs as they arrive. A failure
    // to start it is reported as an error event, like any other error.
    let run_events = events.clone();
    tokio::task::spawn_blocking(move || {
        if let Err(e) = prepared.run(SseChannel {
            events: run_events.clone(),
        }) {
            let _ = run_events.blocking_send(Ok(error_event(e.to_string())));
        }
    });
    tokio::spawn(feed(body, inputs, events));

    Sse::new(ReceiverStream::new(sent))
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// Frames the request body into lines as it arrives and feeds each to the inference.
async fn feed(body: Body, inputs: mpsc::Sender<Value>, events: Events) {
    let mut data = body.into_data_stream();
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = data.next().await {
        let Ok(chunk) = chunk else {
            break; // client disconnected mid-body
        };
        buf.extend_from_slice(&chunk);
        while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = buf.drain(..=pos).collect();
            if !feed_line(&line, &inputs, &events).await {
                return;
            }
        }
    }
    // A last line without a trailing newline.
    let _ = feed_line(&buf, &inputs, &events).await;
}

/// Parses one line as JSON and sends it to the inference. Returns `false`, to stop feeding, when
/// the line is not JSON or the inference has stopped taking input. Blank lines are skipped.
async fn feed_line(line: &[u8], inputs: &mpsc::Sender<Value>, events: &Events) -> bool {
    let line = line.trim_ascii();
    if line.is_empty() {
        return true;
    }
    match serde_json::from_slice(line) {
        Ok(input) => inputs.send(input).is_ok(),
        Err(e) => {
            let _ = events
                .send(Ok(error_event(format!("invalid JSON: {e}"))))
                .await;
            false
        }
    }
}

fn error_event(message: String) -> Event {
    Event::default().event("error").data(message)
}

/// Writes an inference's outputs to the SSE response as they are produced: each output to a
/// `data:` frame, errors to an `error` event, and completion to the terminating `done` event. A
/// failed send means the client disconnected, reported as [`OutputWriterError::Cancelled`] so the
/// inference stops.
struct SseChannel {
    events: Events,
}

impl OutputWriter<Value> for SseChannel {
    fn write(&self, output: Value) -> Result<(), OutputWriterError> {
        self.events
            .blocking_send(Ok(Event::default().data(output.to_string())))
            .map_err(|_| OutputWriterError::Cancelled)
    }

    fn error(&self, error: BoxError) -> Result<(), OutputWriterError> {
        self.events
            .blocking_send(Ok(error_event(error.to_string())))
            .map_err(|_| OutputWriterError::Cancelled)
    }

    fn finish(&self, _duration: std::time::Duration) {
        let _ = self
            .events
            .blocking_send(Ok(Event::default().event("done").data("")));
    }
}
