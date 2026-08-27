//! This module provides utilities for downloading artifact files from any source to any target bundle sink.
//!
//! Downloaded files are validated against expected sizes and checksums when provided, and the download process can be customized with any implementation of the FileTransferClient trait (e.g. for custom HTTP clients, authentication, retries, etc).

use std::collections::HashSet;
use std::io::Read;

use sha2::Digest;

use crate::bundle::BundleSink;
use crate::tools::path::normalize_bundle_path;
use crate::tools::validation::normalize_checksum;
use crate::{FileTransferClient, ReqwestTransferClient, TransferObserver};

/// Errors that can occur during artifact file downloads.
#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    /// The caller cancelled while a file was being transferred.
    #[error("download cancelled while transferring {rel_path}")]
    Cancelled { rel_path: String },
    /// Errors from the transfer client (e.g. network errors, HTTP errors).
    #[error("transfer error for {rel_path}: {source}")]
    Transfer {
        rel_path: String,
        #[source]
        source: crate::transfer::TransferError,
    },
    /// Errors related to file size mismatches after download.
    #[error("size mismatch for {path}: expected {expected} bytes, got {actual} bytes")]
    SizeMismatch {
        path: String,
        expected: u64,
        actual: u64,
    },
    /// Errors related to checksum mismatches after download.
    #[error("checksum mismatch for {path}: expected {expected}, got {actual}")]
    ChecksumMismatch {
        path: String,
        expected: String,
        actual: String,
    },
    /// Errors related to invalid checksums (e.g. non-hex, wrong length).
    #[error("invalid checksum: {0}")]
    InvalidChecksum(String),
    /// Errors related to invalid relative paths (e.g. empty, duplicates, unsafe).
    #[error("invalid path: {0}")]
    InvalidPath(String),
    /// Errors from the target bundle sink (e.g. file system errors).
    #[error("target error: {0}")]
    TargetError(String),
}

/// Generic download descriptor for any model artifact file.
#[derive(Debug, Clone)]
pub struct ArtifactDownloadFile {
    pub rel_path: String,
    pub url: String,
    /// Optional expected file size in bytes.
    pub size_bytes: Option<u64>,
    /// Optional expected SHA-256 checksum.
    pub checksum: Option<String>,
}

/// Download artifact files into any bundle sink implementation.
pub fn download_artifacts_to_sink<S: BundleSink>(
    sink: &mut S,
    files: &[ArtifactDownloadFile],
) -> Result<(), DownloadError> {
    let client = ReqwestTransferClient::new();
    download_artifacts_to_sink_with_client(&client, sink, files)
}

/// Download artifact files into any bundle sink implementation using a custom transfer client.
pub fn download_artifacts_to_sink_with_client<FTC: FileTransferClient, S: BundleSink>(
    client: &FTC,
    sink: &mut S,
    files: &[ArtifactDownloadFile],
) -> Result<(), DownloadError> {
    download_artifacts_to_sink_with_client_and_observer(client, sink, files, &mut ())
}

/// Download artifact files into any bundle sink implementation using a custom transfer client,
/// reporting progress to an observer.
pub fn download_artifacts_to_sink_with_client_and_observer<
    FTC: FileTransferClient,
    S: BundleSink,
    O: TransferObserver,
>(
    client: &FTC,
    sink: &mut S,
    files: &[ArtifactDownloadFile],
    observer: &mut O,
) -> Result<(), DownloadError> {
    let files = validated_download_files(files)?;
    for (rel_path, file) in files {
        if observer.is_cancelled() {
            return Err(DownloadError::Cancelled { rel_path });
        }

        let reader = client
            .get_reader(&file.url)
            .map_err(|e| DownloadError::Transfer {
                rel_path: rel_path.clone(),
                source: e,
            })?;
        if observer.is_cancelled() {
            return Err(DownloadError::Cancelled { rel_path });
        }

        observer.file_started(&rel_path, file.size_bytes);
        let mut verifying_reader = VerifyingReader::new(reader, &rel_path, observer);

        let sink_result = sink.put_file(&rel_path, &mut verifying_reader);
        if verifying_reader.cancelled() {
            return Err(DownloadError::Cancelled { rel_path });
        }
        sink_result.map_err(DownloadError::TargetError)?;

        let (total, digest) = verifying_reader.finish();
        validate_download(
            &rel_path,
            total,
            digest,
            file.size_bytes,
            file.checksum.as_deref(),
        )?;
        observer.file_completed(&rel_path, total);
    }

    Ok(())
}

fn validated_download_files(
    files: &[ArtifactDownloadFile],
) -> Result<Vec<(String, &ArtifactDownloadFile)>, DownloadError> {
    let mut seen = HashSet::with_capacity(files.len());
    let mut out = Vec::with_capacity(files.len());
    for file in files {
        let rel_path = normalize_bundle_path(&file.rel_path);
        if rel_path.is_empty() {
            return Err(DownloadError::InvalidPath(
                "empty relative artifact path".to_string(),
            ));
        }
        if !seen.insert(rel_path.clone()) {
            return Err(DownloadError::InvalidPath(format!(
                "duplicate relative artifact path: {rel_path}"
            )));
        }

        out.push((rel_path, file));
    }

    Ok(out)
}

struct VerifyingReader<'a, R: Read, O: TransferObserver> {
    inner: R,
    hasher: sha2::Sha256,
    total: u64,
    rel_path: &'a str,
    observer: &'a mut O,
    cancelled: bool,
}

impl<'a, R: Read, O: TransferObserver> VerifyingReader<'a, R, O> {
    fn new(inner: R, rel_path: &'a str, observer: &'a mut O) -> Self {
        Self {
            inner,
            hasher: sha2::Sha256::new(),
            total: 0,
            rel_path,
            observer,
            cancelled: false,
        }
    }

    fn cancelled(&self) -> bool {
        self.cancelled
    }

    fn finish(self) -> (u64, String) {
        (self.total, format!("{:x}", self.hasher.finalize()))
    }
}

impl<R: Read, O: TransferObserver> Read for VerifyingReader<'_, R, O> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.observer.is_cancelled() {
            self.cancelled = true;
            return Err(cancelled_io_error());
        }

        let read = match self.inner.read(buf) {
            Ok(read) => read,
            Err(_) if self.observer.is_cancelled() => {
                self.cancelled = true;
                return Err(cancelled_io_error());
            }
            Err(error) => return Err(error),
        };
        if self.observer.is_cancelled() {
            self.cancelled = true;
            return Err(cancelled_io_error());
        }

        self.hasher.update(&buf[..read]);
        self.total += read as u64;
        if read > 0 {
            self.observer.file_progress(self.rel_path, self.total);
            if self.observer.is_cancelled() {
                self.cancelled = true;
                return Err(cancelled_io_error());
            }
        }
        Ok(read)
    }
}

fn cancelled_io_error() -> std::io::Error {
    std::io::Error::other("artifact download cancelled")
}

fn validate_download(
    rel_path: &str,
    total: u64,
    digest: String,
    expected_size: Option<u64>,
    expected_checksum: Option<&str>,
) -> Result<(), DownloadError> {
    if let Some(expected_size) = expected_size {
        if total != expected_size {
            return Err(DownloadError::SizeMismatch {
                path: rel_path.to_string(),
                expected: expected_size,
                actual: total,
            });
        }
    }

    if let Some(expected_checksum) = expected_checksum {
        let expected_checksum =
            normalize_checksum(expected_checksum).map_err(DownloadError::InvalidChecksum)?;
        if digest != expected_checksum {
            return Err(DownloadError::ChecksumMismatch {
                path: rel_path.to_string(),
                expected: expected_checksum,
                actual: digest,
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundle::InMemoryBundleSources;
    use crate::transfer::TransferError;
    use std::collections::HashMap;
    use std::io::{Cursor, Read};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Clone)]
    struct MockClient {
        files: Arc<HashMap<String, Vec<u8>>>,
    }

    impl MockClient {
        fn new(files: HashMap<String, Vec<u8>>) -> Self {
            Self {
                files: Arc::new(files),
            }
        }
    }

    impl FileTransferClient for MockClient {
        fn put_reader<R: Read + Send + 'static>(
            &self,
            _url: &str,
            mut reader: R,
            _size_bytes: u64,
        ) -> Result<(), TransferError> {
            let mut buf = Vec::new();
            reader
                .read_to_end(&mut buf)
                .map_err(|e| TransferError::Transport(e.to_string()))?;
            Ok(())
        }

        fn get_reader(&self, url: &str) -> Result<Box<dyn Read + Send>, TransferError> {
            let bytes = self
                .files
                .get(url)
                .ok_or_else(|| TransferError::Transport(format!("missing url in mock: {url}")))?;
            Ok(Box::new(Cursor::new(bytes.clone())))
        }
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        let mut hasher = sha2::Sha256::new();
        hasher.update(bytes);
        format!("{:x}", hasher.finalize())
    }

    #[test]
    fn downloads_to_sink_and_validates_checksum_and_size() {
        let data = b"hello world".to_vec();
        let checksum = sha256_hex(&data);
        let mut sink = InMemoryBundleSources::new();
        let client = MockClient::new(HashMap::from([("mock://f1".to_string(), data.clone())]));
        let files = vec![ArtifactDownloadFile {
            rel_path: "weights.bin".to_string(),
            url: "mock://f1".to_string(),
            size_bytes: Some(data.len() as u64),
            checksum: Some(checksum),
        }];

        download_artifacts_to_sink_with_client(&client, &mut sink, &files)
            .expect("download should succeed");

        assert_eq!(sink.len(), 1);
        assert_eq!(sink.files()[0].dest_path(), "weights.bin");
        assert_eq!(sink.files()[0].source(), data);
    }

    #[test]
    fn rejects_duplicate_relative_paths() {
        let client = MockClient::new(HashMap::new());
        let mut sink = InMemoryBundleSources::new();
        let files = vec![
            ArtifactDownloadFile {
                rel_path: "a.bin".to_string(),
                url: "mock://a".to_string(),
                size_bytes: None,
                checksum: None,
            },
            ArtifactDownloadFile {
                rel_path: "a.bin".to_string(),
                url: "mock://b".to_string(),
                size_bytes: None,
                checksum: None,
            },
        ];

        let err = download_artifacts_to_sink_with_client(&client, &mut sink, &files)
            .expect_err("duplicate paths should fail");

        match err {
            DownloadError::InvalidPath(msg) => assert!(msg.contains("duplicate")),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn fails_on_checksum_mismatch() {
        let data = b"payload".to_vec();
        let mut sink = InMemoryBundleSources::new();
        let client = MockClient::new(HashMap::from([("mock://f2".to_string(), data.clone())]));
        let files = vec![ArtifactDownloadFile {
            rel_path: "params.bin".to_string(),
            url: "mock://f2".to_string(),
            size_bytes: Some(data.len() as u64),
            checksum: Some("00".repeat(32)),
        }];

        let err = download_artifacts_to_sink_with_client(&client, &mut sink, &files)
            .expect_err("checksum mismatch should fail");

        match err {
            DownloadError::ChecksumMismatch { path, .. } => assert_eq!(path, "params.bin"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn fails_on_size_mismatch() {
        let data = b"payload".to_vec();
        let mut sink = InMemoryBundleSources::new();
        let client = MockClient::new(HashMap::from([("mock://f3".to_string(), data.clone())]));
        let files = vec![ArtifactDownloadFile {
            rel_path: "params.bin".to_string(),
            url: "mock://f3".to_string(),
            size_bytes: Some((data.len() as u64) + 1),
            checksum: None,
        }];

        let err = download_artifacts_to_sink_with_client(&client, &mut sink, &files)
            .expect_err("size mismatch should fail");

        match err {
            DownloadError::SizeMismatch { path, .. } => assert_eq!(path, "params.bin"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[derive(Default)]
    struct RecordingObserver {
        started: Vec<(String, Option<u64>)>,
        progress: Vec<(String, u64)>,
        completed: Vec<(String, u64)>,
    }

    impl TransferObserver for RecordingObserver {
        fn file_started(&mut self, rel_path: &str, expected_bytes: Option<u64>) {
            self.started.push((rel_path.to_string(), expected_bytes));
        }

        fn file_progress(&mut self, rel_path: &str, downloaded_bytes: u64) {
            self.progress.push((rel_path.to_string(), downloaded_bytes));
        }

        fn file_completed(&mut self, rel_path: &str, downloaded_bytes: u64) {
            self.completed
                .push((rel_path.to_string(), downloaded_bytes));
        }
    }

    #[test]
    fn reports_progress_for_every_file() {
        let first = b"hello world".to_vec();
        let second = b"a slightly longer payload".to_vec();
        let mut sink = InMemoryBundleSources::new();
        let client = MockClient::new(HashMap::from([
            ("mock://f1".to_string(), first.clone()),
            ("mock://f2".to_string(), second.clone()),
        ]));
        let files = vec![
            ArtifactDownloadFile {
                rel_path: "a.bin".to_string(),
                url: "mock://f1".to_string(),
                size_bytes: Some(first.len() as u64),
                checksum: None,
            },
            ArtifactDownloadFile {
                rel_path: "b.bin".to_string(),
                url: "mock://f2".to_string(),
                size_bytes: None,
                checksum: None,
            },
        ];
        let mut observer = RecordingObserver::default();

        download_artifacts_to_sink_with_client_and_observer(
            &client,
            &mut sink,
            &files,
            &mut observer,
        )
        .expect("download should succeed");

        assert_eq!(
            observer.started,
            vec![
                ("a.bin".to_string(), Some(first.len() as u64)),
                ("b.bin".to_string(), None),
            ]
        );
        assert_eq!(
            observer.completed,
            vec![
                ("a.bin".to_string(), first.len() as u64),
                ("b.bin".to_string(), second.len() as u64),
            ]
        );

        // Each file counts from zero, so the totals only ever grow within a file and land on
        // the number of bytes that file actually delivered.
        for (rel_path, size) in [
            ("a.bin", first.len() as u64),
            ("b.bin", second.len() as u64),
        ] {
            let totals: Vec<u64> = observer
                .progress
                .iter()
                .filter(|(path, _)| path == rel_path)
                .map(|(_, total)| *total)
                .collect();

            assert!(!totals.is_empty(), "{rel_path} reported no progress");
            assert!(totals.is_sorted(), "{rel_path} progress went backwards");
            assert_eq!(totals.last().copied(), Some(size));
        }
    }

    /// A file that fails verification was never delivered, so it must not be announced as done.
    #[test]
    fn does_not_complete_a_file_that_fails_verification() {
        let data = b"payload".to_vec();
        let mut sink = InMemoryBundleSources::new();
        let client = MockClient::new(HashMap::from([("mock://f4".to_string(), data.clone())]));
        let files = vec![ArtifactDownloadFile {
            rel_path: "params.bin".to_string(),
            url: "mock://f4".to_string(),
            size_bytes: Some((data.len() as u64) + 1),
            checksum: None,
        }];
        let mut observer = RecordingObserver::default();

        download_artifacts_to_sink_with_client_and_observer(
            &client,
            &mut sink,
            &files,
            &mut observer,
        )
        .expect_err("size mismatch should fail");

        assert_eq!(observer.started.len(), 1);
        assert!(observer.completed.is_empty());
    }

    #[derive(Clone)]
    struct ChunkedClient {
        bytes: Arc<Vec<u8>>,
        consumed: Arc<AtomicUsize>,
    }

    impl FileTransferClient for ChunkedClient {
        fn put_reader<R: Read + Send + 'static>(
            &self,
            _url: &str,
            _reader: R,
            _size_bytes: u64,
        ) -> Result<(), TransferError> {
            unreachable!("the cancellation test only downloads")
        }

        fn get_reader(&self, _url: &str) -> Result<Box<dyn Read + Send>, TransferError> {
            Ok(Box::new(ChunkedReader {
                bytes: Arc::clone(&self.bytes),
                consumed: Arc::clone(&self.consumed),
                offset: 0,
            }))
        }
    }

    struct ChunkedReader {
        bytes: Arc<Vec<u8>>,
        consumed: Arc<AtomicUsize>,
        offset: usize,
    }

    impl Read for ChunkedReader {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            if self.offset == self.bytes.len() {
                return Ok(0);
            }

            let read = 4.min(buffer.len()).min(self.bytes.len() - self.offset);
            buffer[..read].copy_from_slice(&self.bytes[self.offset..self.offset + read]);
            self.offset += read;
            self.consumed.fetch_add(read, Ordering::SeqCst);
            Ok(read)
        }
    }

    #[derive(Default)]
    struct CancellingObserver {
        cancelled: bool,
        completed: bool,
    }

    impl TransferObserver for CancellingObserver {
        fn is_cancelled(&self) -> bool {
            self.cancelled
        }

        fn file_progress(&mut self, _rel_path: &str, _downloaded_bytes: u64) {
            self.cancelled = true;
        }

        fn file_completed(&mut self, _rel_path: &str, _downloaded_bytes: u64) {
            self.completed = true;
        }
    }

    #[test]
    fn cancellation_stops_an_active_transfer_before_eof() {
        let bytes = Arc::new(b"a payload spanning several reads".to_vec());
        let consumed = Arc::new(AtomicUsize::new(0));
        let client = ChunkedClient {
            bytes: Arc::clone(&bytes),
            consumed: Arc::clone(&consumed),
        };
        let files = [ArtifactDownloadFile {
            rel_path: "weights.bin".to_string(),
            url: "mock://weights".to_string(),
            size_bytes: Some(bytes.len() as u64),
            checksum: None,
        }];
        let mut sink = InMemoryBundleSources::new();
        let mut observer = CancellingObserver::default();

        let error = download_artifacts_to_sink_with_client_and_observer(
            &client,
            &mut sink,
            &files,
            &mut observer,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            DownloadError::Cancelled { rel_path } if rel_path == "weights.bin"
        ));
        assert_eq!(consumed.load(Ordering::SeqCst), 4);
        assert!(consumed.load(Ordering::SeqCst) < bytes.len());
        assert!(!observer.completed);
        assert!(sink.is_empty());
    }
}
