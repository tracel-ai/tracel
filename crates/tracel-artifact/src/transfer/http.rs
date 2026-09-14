use std::io;
use std::ops::Range;

use bytes::Bytes;
use futures::{Stream, TryStreamExt};
use reqwest::header::{CONTENT_LENGTH, CONTENT_RANGE, RANGE};
use reqwest::{StatusCode, header::HeaderMap};
use tracel_task::MaybeSend;

use super::ranged::{self, RangePlan, RangeResponse, RangeSource};
use super::timeout_worth_allowing_a_transfer_of;
use super::{ByteStream, TransferClient, TransferError, transport_failure};

/// Transfers bytes over HTTP, on every target.
///
/// Large downloads are fetched as concurrent ranges when the source serves them. Each request's
/// deadline follows the size it moves, since a transfer's duration is set by the caller's
/// bandwidth rather than by any fixed budget.
#[derive(Clone)]
pub struct HttpTransferClient {
    http: reqwest::Client,
    plan: RangePlan,
}

impl HttpTransferClient {
    pub fn new() -> Self {
        let builder = reqwest::Client::builder();
        #[cfg(not(target_arch = "wasm32"))]
        let builder = builder.connect_timeout(super::CONNECT_TIMEOUT);
        let http = builder
            .build()
            .expect("failed to build the HTTP transfer client");

        Self::with_client(http)
    }

    pub fn with_client(http: reqwest::Client) -> Self {
        Self {
            http,
            plan: RangePlan::default(),
        }
    }
}

impl Default for HttpTransferClient {
    fn default() -> Self {
        Self::new()
    }
}

impl RangeSource for HttpTransferClient {
    type Body = ByteStream;

    async fn whole(
        &self,
        url: &str,
        expected_size_bytes: Option<u64>,
    ) -> Result<ByteStream, TransferError> {
        let response = self
            .http
            .get(url)
            .timeout(timeout_worth_allowing_a_transfer_of(expected_size_bytes))
            .send()
            .await
            .map_err(|error| transport_failure(&error))?;

        Ok(body(succeeded(response)?))
    }

    async fn range(
        &self,
        url: &str,
        offset: u64,
        length: u64,
    ) -> Result<Option<RangeResponse<ByteStream>>, TransferError> {
        let last = offset + length.saturating_sub(1);
        let response = self
            .http
            .get(url)
            .header(RANGE, format!("bytes={offset}-{last}"))
            .timeout(timeout_worth_allowing_a_transfer_of(Some(length)))
            .send()
            .await
            .map_err(|error| transport_failure(&error))?;

        if response.status() == StatusCode::PARTIAL_CONTENT {
            let (range, total_size) = parse_content_range(response.headers())?;
            return Ok(Some(RangeResponse {
                body: body(response),
                range,
                total_size,
            }));
        }
        // Any other success means the source ignored the Range header.
        if response.status().is_success() {
            return Ok(None);
        }
        succeeded(response).map(|_| None)
    }
}

impl TransferClient for HttpTransferClient {
    type Body = ByteStream;

    async fn get(
        &self,
        url: &str,
        expected_size_bytes: Option<u64>,
    ) -> Result<ByteStream, TransferError> {
        ranged::open(self, url, expected_size_bytes, &self.plan).await
    }

    async fn put<B>(&self, url: &str, body: B, size_bytes: u64) -> Result<(), TransferError>
    where
        B: Stream<Item = Result<Bytes, io::Error>> + MaybeSend + 'static,
    {
        #[cfg(not(target_arch = "wasm32"))]
        let body = reqwest::Body::wrap_stream(body);
        // A browser cannot stream a request body, so the upload is assembled first.
        #[cfg(target_arch = "wasm32")]
        let body = body
            .map_ok(|chunk| chunk.to_vec())
            .try_concat()
            .await
            .map_err(|error| transport_failure(&error))?;

        let response = self
            .http
            .put(url)
            .header(CONTENT_LENGTH, size_bytes)
            .timeout(timeout_worth_allowing_a_transfer_of(Some(size_bytes)))
            .body(body)
            .send()
            .await
            .map_err(|error| transport_failure(&error))?;

        succeeded(response).map(drop)
    }
}

fn body(response: reqwest::Response) -> ByteStream {
    Box::pin(
        response
            .bytes_stream()
            .map_err(|error| transport_failure(&error)),
    )
}

fn succeeded(response: reqwest::Response) -> Result<reqwest::Response, TransferError> {
    let status = response.status();
    match response.error_for_status() {
        Ok(response) if status.is_success() => Ok(response),
        Ok(_) => Err(TransferError::Transport(format!(
            "unexpected response status {status}"
        ))),
        Err(error) => Err(transport_failure(&error)),
    }
}

fn parse_content_range(headers: &HeaderMap) -> Result<(Range<u64>, u64), TransferError> {
    let value = headers
        .get(CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| TransferError::Transport("partial response omitted Content-Range".into()))?;
    parse_content_range_value(value)
}

fn parse_content_range_value(value: &str) -> Result<(Range<u64>, u64), TransferError> {
    let invalid = || TransferError::Transport(format!("invalid Content-Range header: {value}"));
    let spec = value.strip_prefix("bytes ").ok_or_else(invalid)?;
    let (range, total) = spec.split_once('/').ok_or_else(invalid)?;
    let (start, end) = range.split_once('-').ok_or_else(invalid)?;
    let start = start.parse::<u64>().map_err(|_| invalid())?;
    let end = end.parse::<u64>().map_err(|_| invalid())?;
    let total = total.parse::<u64>().map_err(|_| invalid())?;
    let end = end.checked_add(1).ok_or_else(invalid)?;
    if start >= end || end > total {
        return Err(invalid());
    }
    Ok((start..end, total))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::super::test_server::TestServer;
    use super::*;

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    fn client(range_bytes: u64) -> HttpTransferClient {
        HttpTransferClient {
            http: reqwest::Client::new(),
            plan: RangePlan {
                workers: 2,
                range_bytes,
                attempts: 2,
                backoff: Duration::from_millis(1),
            },
        }
    }

    fn file(len: usize) -> Vec<u8> {
        (0..len).map(|byte| (byte % 251) as u8).collect()
    }

    async fn read_all(body: ByteStream) -> Vec<u8> {
        body.map_ok(|chunk| chunk.to_vec())
            .try_concat()
            .await
            .unwrap()
    }

    #[test]
    fn downloads_a_file_that_fits_one_range_in_one_request() {
        let server = TestServer::serve(file(500), true);
        let client = client(1024);

        let read = runtime().block_on(async {
            let body = client.get(&server.url("/file"), Some(500)).await.unwrap();
            read_all(body).await
        });

        assert_eq!(read, file(500));
        let received = server.received();
        assert_eq!(received.len(), 1);
        assert!(received[0].header("range").is_none());
    }

    #[test]
    fn downloads_a_large_file_as_ranges_and_reassembles_it() {
        let server = TestServer::serve(file(3 * 1024 + 100), true);
        let client = client(1024);

        let read = runtime().block_on(async {
            let body = client
                .get(&server.url("/file"), Some(3 * 1024 + 100))
                .await
                .unwrap();
            read_all(body).await
        });

        assert_eq!(read, file(3 * 1024 + 100));
        let ranges: Vec<_> = server
            .received()
            .iter()
            .filter_map(|request| request.header("range").map(str::to_string))
            .collect();
        assert_eq!(
            ranges,
            [
                "bytes=0-1023",
                "bytes=1024-2047",
                "bytes=2048-3071",
                "bytes=3072-3171"
            ]
        );
    }

    #[test]
    fn a_source_without_range_support_is_downloaded_whole() {
        let server = TestServer::serve(file(3 * 1024), false);
        let client = client(1024);

        let read = runtime().block_on(async {
            let body = client
                .get(&server.url("/file"), Some(3 * 1024))
                .await
                .unwrap();
            read_all(body).await
        });

        assert_eq!(read, file(3 * 1024));
    }

    #[test]
    fn a_missing_file_fails_before_any_body_is_read() {
        let server = TestServer::serve(file(10), true);
        let client = client(1024);

        let error = runtime()
            .block_on(client.get(&server.url("/missing"), Some(10)))
            .err()
            .expect("404 is a transport failure");

        assert!(error.to_string().contains("404"), "{error}");
    }

    #[test]
    fn uploads_announce_their_length_rather_than_chunking() {
        let server = TestServer::serve(Vec::new(), true);
        let client = client(1024);
        let payload = file(70 * 1024);
        let body = super::super::reader_stream(io::Cursor::new(payload.clone()));

        runtime()
            .block_on(client.put(&server.url("/upload"), body, payload.len() as u64))
            .unwrap();

        let received = server.received();
        assert_eq!(received[0].method, "PUT");
        assert_eq!(
            received[0].header("content-length"),
            Some(payload.len().to_string().as_str())
        );
        assert!(received[0].header("transfer-encoding").is_none());
        assert_eq!(received[0].body, payload);
    }

    #[test]
    fn parses_content_range_boundaries() {
        let (range, total) =
            parse_content_range_value("bytes 10-19/100").expect("valid Content-Range");

        assert_eq!(range, 10..20);
        assert_eq!(total, 100);
    }

    #[test]
    fn rejects_content_range_without_a_known_total() {
        let error = parse_content_range_value("bytes 10-19/*").expect_err("unknown total");

        assert!(error.to_string().contains("invalid Content-Range"));
    }

    #[test]
    fn rejects_content_range_past_its_total() {
        let error = parse_content_range_value("bytes 90-100/100").expect_err("range past total");

        assert!(error.to_string().contains("invalid Content-Range"));
    }
}
