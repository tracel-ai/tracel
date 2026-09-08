use std::io::Read;
use std::time::Duration;

const TRANSFER_SECONDS_ALLOWED_PER_MEGABYTE: u64 = 10;

const MINIMUM_TRANSFER_TIMEOUT: Duration = Duration::from_secs(60);

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

fn timeout_worth_allowing_a_transfer_of(size_bytes: Option<u64>) -> Duration {
    const BYTES_PER_MEGABYTE: u64 = 1024 * 1024;

    let Some(size_bytes) = size_bytes else {
        return Duration::from_secs(60 * 60);
    };

    let megabytes = size_bytes.div_ceil(BYTES_PER_MEGABYTE);
    let allowed =
        Duration::from_secs(megabytes.saturating_mul(TRANSFER_SECONDS_ALLOWED_PER_MEGABYTE));

    allowed.max(MINIMUM_TRANSFER_TIMEOUT)
}

fn transport_failure(error: &dyn std::error::Error) -> TransferError {
    let mut described = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        described.push_str(": ");
        described.push_str(&cause.to_string());
        source = cause.source();
    }

    TransferError::Transport(described)
}

mod ranged;

use ranged::{RangeResponse, RangeSource, RangeSourceError};

/// Watches a transfer as it runs, and can stop it.
///
/// Every method defaults to doing nothing, so an implementation only has to define the events it
/// cares about. Callbacks run on the transferring thread and block it, so an implementation that
/// does real work should hand it off.
pub trait TransferObserver {
    /// Returns whether the active transfer should stop.
    ///
    /// Polled at file, part, and reader boundaries. Implementations should make this query cheap
    /// and may update its result from another thread or from a progress callback.
    fn is_cancelled(&self) -> bool {
        false
    }

    /// A file is about to be transferred. `total_bytes` is absent when nothing announced the
    /// size, as for an artifact published without a manifest.
    fn file_started(&mut self, rel_path: &str, total_bytes: Option<u64>) {
        let _ = (rel_path, total_bytes);
    }

    /// Bytes have moved for the file being transferred. `transferred_bytes` is the running total
    /// for that file, not an increment.
    fn file_progress(&mut self, rel_path: &str, transferred_bytes: u64) {
        let _ = (rel_path, transferred_bytes);
    }

    /// A file has been transferred in full.
    fn file_completed(&mut self, rel_path: &str, transferred_bytes: u64) {
        let _ = (rel_path, transferred_bytes);
    }
}

/// Transfers no one is watching.
impl TransferObserver for () {}

/// So a borrowed observer can be handed to code that wants to own one.
impl<O: TransferObserver + ?Sized> TransferObserver for &mut O {
    fn is_cancelled(&self) -> bool {
        (**self).is_cancelled()
    }

    fn file_started(&mut self, rel_path: &str, total_bytes: Option<u64>) {
        (**self).file_started(rel_path, total_bytes);
    }

    fn file_progress(&mut self, rel_path: &str, transferred_bytes: u64) {
        (**self).file_progress(rel_path, transferred_bytes);
    }

    fn file_completed(&mut self, rel_path: &str, transferred_bytes: u64) {
        (**self).file_completed(rel_path, transferred_bytes);
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TransferError {
    #[error("Transport error: {0}")]
    Transport(String),
}

/// Generic client interface used for uploading and downloading files, abstracting over the underlying HTTP client or other transport mechanism.
pub trait FileTransferClient: Clone + Send + Sync + 'static {
    /// Upload data from a reader to the given URL with known size.
    fn put_reader<R: Read + Send + 'static>(
        &self,
        url: &str,
        reader: R,
        size_bytes: u64,
    ) -> Result<(), TransferError>;

    /// Download data from the given URL as a reader.
    ///
    /// `expected_size` is a transport hint. Implementations may ignore it and read the whole
    /// source, or use it to select a more efficient transfer strategy.
    fn get_reader(
        &self,
        url: &str,
        expected_size: Option<u64>,
    ) -> Result<Box<dyn Read + Send>, TransferError>;
}

/// Reqwest-based transfer client.
#[derive(Clone)]
pub struct ReqwestTransferClient {
    http: reqwest::blocking::Client,
}

impl ReqwestTransferClient {
    /// A transfer is one request whose duration is set by the caller's
    /// bandwidth, so the deadline is set per request from the size being moved
    /// rather than once here. Reqwest's own default is 30 seconds, which no
    /// model larger than a few megabytes survives.
    pub fn new() -> Self {
        let http = reqwest::blocking::Client::builder()
            .timeout(None)
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .expect("failed to build the HTTP transfer client");

        Self { http }
    }

    pub fn with_client(http: reqwest::blocking::Client) -> Self {
        Self { http }
    }

    fn get_whole_reader(&self, url: &str) -> Result<Box<dyn Read + Send>, TransferError> {
        let response = self
            .http
            .get(url)
            .send()
            .map_err(|e| TransferError::Transport(e.to_string()))?;

        if !response.status().is_success() {
            return Err(TransferError::Transport(
                response.error_for_status().err().unwrap().to_string(),
            ));
        }

        Ok(Box::new(response))
    }
}

impl Default for ReqwestTransferClient {
    fn default() -> Self {
        Self::new()
    }
}

impl FileTransferClient for ReqwestTransferClient {
    fn put_reader<R: Read + Send + 'static>(
        &self,
        url: &str,
        reader: R,
        size_bytes: u64,
    ) -> Result<(), TransferError> {
        let body = reqwest::blocking::Body::sized(reader, size_bytes);
        let response = self
            .http
            .put(url)
            .timeout(timeout_worth_allowing_a_transfer_of(Some(size_bytes)))
            .body(body)
            .send()
            .map_err(|error| transport_failure(&error))?;

        if !response.status().is_success() {
            return Err(transport_failure(
                &response.error_for_status().err().unwrap(),
            ));
        }

        Ok(())
    }

    fn get_reader(
        &self,
        url: &str,
        expected_size: Option<u64>,
    ) -> Result<Box<dyn Read + Send>, TransferError> {
        ranged::open_for_download(self, url, expected_size)
    }
}

impl RangeSource for ReqwestTransferClient {
    fn get_whole_reader(&self, url: &str) -> Result<Box<dyn Read + Send>, TransferError> {
        self.get_whole_reader(url)
    }

    fn get_range(
        &self,
        url: &str,
        offset: u64,
        length: u64,
    ) -> Result<RangeResponse, RangeSourceError> {
        let last = offset + length.saturating_sub(1);
        let response = self
            .http
            .get(url)
            .header(reqwest::header::RANGE, format!("bytes={offset}-{last}"))
            .send()
            .map_err(|e| TransferError::Transport(e.to_string()))?;

        if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
            let (range, total_size) = parse_content_range(&response)?;
            return Ok(RangeResponse {
                reader: Box::new(response),
                range,
                total_size,
            });
        }
        // Any other success is the whole file: the source ignored the header.
        if response.status().is_success() {
            return Err(RangeSourceError::Unsupported);
        }
        Err(TransferError::Transport(response.error_for_status().err().unwrap().to_string()).into())
    }
}

fn parse_content_range(
    response: &reqwest::blocking::Response,
) -> Result<(std::ops::Range<u64>, u64), TransferError> {
    let value = response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| TransferError::Transport("partial response omitted Content-Range".into()))?;
    parse_content_range_value(value)
}

fn parse_content_range_value(value: &str) -> Result<(std::ops::Range<u64>, u64), TransferError> {
    let value = value.strip_prefix("bytes ").ok_or_else(|| {
        TransferError::Transport(format!("invalid Content-Range header: {value}"))
    })?;
    let (range, total) = value.split_once('/').ok_or_else(|| {
        TransferError::Transport(format!("invalid Content-Range header: {value}"))
    })?;
    let (start, end) = range.split_once('-').ok_or_else(|| {
        TransferError::Transport(format!("invalid Content-Range header: {value}"))
    })?;
    let start = start
        .parse::<u64>()
        .map_err(|_| TransferError::Transport(format!("invalid Content-Range header: {value}")))?;
    let end = end
        .parse::<u64>()
        .map_err(|_| TransferError::Transport(format!("invalid Content-Range header: {value}")))?;
    let total = total
        .parse::<u64>()
        .map_err(|_| TransferError::Transport(format!("invalid Content-Range header: {value}")))?;
    let end = end.checked_add(1).ok_or_else(|| {
        TransferError::Transport(format!("invalid Content-Range header: {value}"))
    })?;
    if start >= end || end > total {
        return Err(TransferError::Transport(format!(
            "invalid Content-Range header: {value}"
        )));
    }
    Ok((start..end, total))
}

#[cfg(test)]
mod tests {
    use super::parse_content_range_value;

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
