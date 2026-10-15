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
use tracing::warn;
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
    pub fn open_events(
        &self,
        register: &RegisterRunner,
    ) -> Result<RunnerEventStream<BufReader<Response>>, ClientError> {
        let response = self
            .events_stream_client
            .post(self.join("runners/events"))
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .json(register)
            .send()?;
        let response = ensure_success(response)?;
        Ok(RunnerEventStream::new(BufReader::new(response)))
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

/// Blocking iterator over the events of an open runner stream, read from `R`.
///
/// Skips a frame that does not decode, with a warning, and ends when the station closes the stream
/// or reading it fails; the session is over either way.
pub struct RunnerEventStream<R> {
    lines: std::io::Lines<R>,
    parser: SseParser,
}

impl<R: BufRead> RunnerEventStream<R> {
    fn new(reader: R) -> Self {
        Self {
            lines: reader.lines(),
            parser: SseParser::default(),
        }
    }
}

impl<R: BufRead> Iterator for RunnerEventStream<R> {
    type Item = RunnerEvent;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let line = match self.lines.next()? {
                Ok(line) => line,
                Err(error) => {
                    warn!(%error, "the event stream failed");
                    return None;
                }
            };
            let Some(frame) = self.parser.push_line(&line) else {
                continue;
            };
            match RunnerEvent::decode(&frame.event, &frame.data) {
                Ok(Some(event)) => return Some(event),
                Ok(None) => {}
                Err(error) => warn!(event = %frame.event, %error, "skipped a malformed event"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{self, Read};

    use serde_json::json;

    use super::*;

    struct FailingOnce(bool);

    impl Read for FailingOnce {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            if std::mem::replace(&mut self.0, true) {
                return Ok(0);
            }
            Err(io::Error::other("connection reset"))
        }
    }

    fn frame(event: &str, data: &str) -> String {
        format!("event: {event}\ndata: {data}\n\n")
    }

    fn registered(runner_id: Uuid) -> String {
        frame("registered", &json!({ "runner_id": runner_id }).to_string())
    }

    fn events(stream: impl Read) -> Vec<RunnerEvent> {
        RunnerEventStream::new(BufReader::new(stream)).collect()
    }

    #[test]
    fn a_frame_whose_data_does_not_decode_is_skipped() {
        let runner_id = Uuid::new_v4();
        let stream = frame("job", "not json") + &registered(runner_id);

        assert_eq!(
            events(stream.as_bytes()),
            [RunnerEvent::Registered { runner_id }]
        );
    }

    #[test]
    fn a_failed_read_ends_the_stream() {
        let runner_id = Uuid::new_v4();
        let registered = registered(runner_id);
        let cancel = frame("cancel", &json!({ "job_id": Uuid::new_v4() }).to_string());
        let stream = registered
            .as_bytes()
            .chain(FailingOnce(false))
            .chain(cancel.as_bytes());

        assert_eq!(events(stream), [RunnerEvent::Registered { runner_id }]);
    }
}
