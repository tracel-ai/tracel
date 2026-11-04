//! HTTP/SSE client for the station's runner endpoints.
//!
//! Self-contained on purpose: the runner protocol (a POST whose response is an SSE stream) does
//! not fit the request/response transport of the general station client.

pub mod protocol;
mod sse;

use std::io::BufRead;
use std::io::BufReader;
use std::time::Duration;

use reqwest::Url;
use reqwest::blocking::{Client, Response};
use uuid::Uuid;

use protocol::{FinishJob, RegisterRunner, RunnerEvent};
use sse::SseParser;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("station returned {status}: {body}")]
    Api {
        status: reqwest::StatusCode,
        body: String,
    },
    #[error("stream io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid event payload: {0}")]
    Decode(#[from] serde_json::Error),
}

impl ClientError {
    pub fn is_permanent(&self) -> bool {
        matches!(
            self,
            ClientError::Api { status, .. }
                if status.is_client_error()
                    && *status != reqwest::StatusCode::REQUEST_TIMEOUT
                    && *status != reqwest::StatusCode::TOO_MANY_REQUESTS
        )
    }
}

#[derive(Debug, Clone)]
pub struct StationRunnerClient {
    base_url: Url,
    /// Streaming client: no overall timeout (the stream lives for the whole session); TCP
    /// keepalive is the backstop that resets half-open connections.
    events_stream_client: Client,
    short_call_client: Client,
}

impl StationRunnerClient {
    pub fn new(base_url: Url) -> Self {
        let events_stream_client = Client::builder()
            .timeout(None)
            .tcp_keepalive(Duration::from_secs(60))
            .build()
            .expect("failed to build HTTP client");
        let short_call_client = Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("failed to build HTTP client");
        Self {
            base_url,
            events_stream_client,
            short_call_client,
        }
    }

    /// Register with the station; the successful response IS the runner's event stream.
    pub fn open_events(&self, register: &RegisterRunner) -> Result<RunnerEventStream, ClientError> {
        let response = self
            .events_stream_client
            .post(self.join("runners/events"))
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .json(register)
            .send()?;
        let response = ensure_success(response)?;
        Ok(RunnerEventStream::new(response))
    }

    pub fn finish_job(&self, job_id: Uuid, finish: &FinishJob) -> Result<(), ClientError> {
        let response = self
            .short_call_client
            .post(self.join(&format!("jobs/{job_id}/finish")))
            .json(finish)
            .send()?;
        ensure_success(response).map(|_| ())
    }

    fn join(&self, path: &str) -> Url {
        self.base_url
            .join("v1/")
            .expect("station base url should accept a path")
            .join(path)
            .expect("runner endpoint paths are valid")
    }
}

fn ensure_success(response: Response) -> Result<Response, ClientError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.text().unwrap_or_default();
    Err(ClientError::Api { status, body })
}

/// Blocking iterator over the events of an open runner stream.
///
/// Yields `None` when the station closes the stream; the session is over either way.
pub struct RunnerEventStream {
    lines: std::io::Lines<BufReader<Response>>,
    parser: SseParser,
}

impl RunnerEventStream {
    fn new(response: Response) -> Self {
        Self {
            lines: BufReader::new(response).lines(),
            parser: SseParser::default(),
        }
    }
}

impl Iterator for RunnerEventStream {
    type Item = Result<RunnerEvent, ClientError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let line = match self.lines.next()? {
                Ok(line) => line,
                Err(e) => return Some(Err(e.into())),
            };
            let Some(frame) = self.parser.push_line(&line) else {
                continue;
            };
            match RunnerEvent::decode(&frame.event, &frame.data) {
                Ok(Some(event)) => return Some(Ok(event)),
                Ok(None) => continue,
                Err(e) => return Some(Err(e.into())),
            }
        }
    }
}
