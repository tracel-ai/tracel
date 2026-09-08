//! This module provides utilities for downloading artifact files from any source to any target bundle sink.
//!
//! Downloaded files are validated against expected sizes and checksums when provided, and the download process can be customized with any implementation of the FileTransferClient trait (e.g. for custom HTTP clients, authentication, retries, etc).

use std::collections::HashSet;
use std::io::{self, Read};

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

/// Transport-neutral metadata for one artifact file.
#[derive(Debug, Clone)]
pub struct ArtifactFile {
    pub rel_path: String,
    /// Optional expected file size in bytes.
    pub size_bytes: Option<u64>,
    /// Optional expected SHA-256 checksum.
    pub checksum: Option<String>,
}

/// Download descriptor for one artifact file.
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
    O: TransferObserver + ?Sized,
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

        let reader =
            client
                .get_reader(&file.url, file.size_bytes)
                .map_err(|e| DownloadError::Transfer {
                    rel_path: rel_path.clone(),
                    source: e,
                })?;
        if observer.is_cancelled() {
            return Err(DownloadError::Cancelled { rel_path });
        }

        let artifact_file = ArtifactFile {
            rel_path,
            size_bytes: file.size_bytes,
            checksum: file.checksum.clone(),
        };
        transfer_reader_to_sink_with_observer(reader, sink, &artifact_file, observer)?;
    }

    Ok(())
}

/// Transfer an already-open artifact reader into a bundle sink.
pub fn transfer_reader_to_sink<R: Read, S: BundleSink>(
    reader: R,
    sink: &mut S,
    file: &ArtifactFile,
) -> Result<(), DownloadError> {
    transfer_reader_to_sink_with_observer(reader, sink, file, &mut ())
}

/// Transfer an already-open artifact reader into a bundle sink, reporting progress to an
/// observer.
pub fn transfer_reader_to_sink_with_observer<
    R: Read,
    S: BundleSink,
    O: TransferObserver + ?Sized,
>(
    reader: R,
    sink: &mut S,
    file: &ArtifactFile,
    observer: &mut O,
) -> Result<(), DownloadError> {
    let rel_path = validated_artifact_path(&file.rel_path)?;
    if observer.is_cancelled() {
        return Err(DownloadError::Cancelled { rel_path });
    }

    observer.file_started(&rel_path, file.size_bytes);
    let mut verifying_reader = VerifyingReader::new(reader, &rel_path, observer);
    let sink_result = sink.put_file(&rel_path, &mut verifying_reader);

    // BundleSink flattens read errors into a string. Inspect the reader before the sink result so
    // cancellation and source failures keep their original classification.
    if verifying_reader.cancelled() {
        return Err(DownloadError::Cancelled { rel_path });
    }
    if let Some(source) = verifying_reader.take_source_error() {
        return Err(DownloadError::Transfer {
            rel_path,
            source: crate::transfer::TransferError::Transport(source.to_string()),
        });
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

    Ok(())
}

fn validated_artifact_path(path: &str) -> Result<String, DownloadError> {
    let rel_path = normalize_bundle_path(path);
    if rel_path.is_empty() {
        return Err(DownloadError::InvalidPath(
            "empty relative artifact path".to_string(),
        ));
    }
    Ok(rel_path)
}

fn validated_download_files(
    files: &[ArtifactDownloadFile],
) -> Result<Vec<(String, &ArtifactDownloadFile)>, DownloadError> {
    let mut seen = HashSet::with_capacity(files.len());
    let mut out = Vec::with_capacity(files.len());
    for file in files {
        let rel_path = validated_artifact_path(&file.rel_path)?;
        if !seen.insert(rel_path.clone()) {
            return Err(DownloadError::InvalidPath(format!(
                "duplicate relative artifact path: {rel_path}"
            )));
        }

        out.push((rel_path, file));
    }

    Ok(out)
}

struct VerifyingReader<'a, R: Read, O: TransferObserver + ?Sized> {
    inner: R,
    hasher: sha2::Sha256,
    total: u64,
    rel_path: &'a str,
    observer: &'a mut O,
    cancelled: bool,
    source_error: Option<io::Error>,
}

impl<'a, R: Read, O: TransferObserver + ?Sized> VerifyingReader<'a, R, O> {
    fn new(inner: R, rel_path: &'a str, observer: &'a mut O) -> Self {
        Self {
            inner,
            hasher: sha2::Sha256::new(),
            total: 0,
            rel_path,
            observer,
            cancelled: false,
            source_error: None,
        }
    }

    fn cancelled(&self) -> bool {
        self.cancelled || self.observer.is_cancelled()
    }

    fn take_source_error(&mut self) -> Option<io::Error> {
        self.source_error.take()
    }

    fn finish(self) -> (u64, String) {
        (self.total, format!("{:x}", self.hasher.finalize()))
    }
}

impl<R: Read, O: TransferObserver + ?Sized> Read for VerifyingReader<'_, R, O> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = loop {
            if self.observer.is_cancelled() {
                self.cancelled = true;
                return Err(cancelled_io_error());
            }

            match self.inner.read(buf) {
                Ok(read) => break read,
                Err(_) if self.observer.is_cancelled() => {
                    self.cancelled = true;
                    return Err(cancelled_io_error());
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    // Keep the source failure separate until the transfer layer can classify it.
                    // The sink only needs a surrogate to stop its copy loop.
                    let surrogate = surrogate_io_error(&error);
                    if self.source_error.is_none() {
                        self.source_error = Some(error);
                    }
                    return Err(surrogate);
                }
            }
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

fn surrogate_io_error(error: &io::Error) -> io::Error {
    error
        .raw_os_error()
        .map(io::Error::from_raw_os_error)
        .unwrap_or_else(|| io::Error::new(error.kind(), error.to_string()))
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
    use crate::bundle::{BundleSink, InMemoryBundleSources};
    use crate::transfer::TransferError;
    use std::collections::HashMap;
    use std::fmt;
    use std::io::{Cursor, Read};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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

        fn get_reader(
            &self,
            url: &str,
            _expected_size_bytes: Option<u64>,
        ) -> Result<Box<dyn Read + Send>, TransferError> {
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
    fn transfers_an_already_open_reader() {
        let data = b"reader-native artifact".to_vec();
        let reader: Box<dyn Read + Send> = Box::new(Cursor::new(data.clone()));
        let file = ArtifactFile {
            rel_path: "weights.bin".to_string(),
            size_bytes: Some(data.len() as u64),
            checksum: Some(sha256_hex(&data)),
        };
        let mut sink = InMemoryBundleSources::new();

        transfer_reader_to_sink(reader, &mut sink, &file)
            .expect("an already-open reader should transfer");

        assert_eq!(sink.len(), 1);
        assert_eq!(sink.files()[0].dest_path(), "weights.bin");
        assert_eq!(sink.files()[0].source(), data);
    }

    struct InterruptedOnceReader {
        interrupted: bool,
        inner: Cursor<Vec<u8>>,
    }

    impl Read for InterruptedOnceReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if !self.interrupted {
                self.interrupted = true;
                return Err(io::ErrorKind::Interrupted.into());
            }
            self.inner.read(buffer)
        }
    }

    #[test]
    fn retries_an_interrupted_source_read() {
        let data = b"eventually readable".to_vec();
        let reader = InterruptedOnceReader {
            interrupted: false,
            inner: Cursor::new(data.clone()),
        };
        let mut sink = InMemoryBundleSources::new();

        transfer_reader_to_sink(reader, &mut sink, &unverified_file())
            .expect("an interrupted read should be retried");

        assert_eq!(sink.files()[0].source(), data);
    }

    #[derive(Debug)]
    struct OriginalSourceError;

    impl fmt::Display for OriginalSourceError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("original source failure")
        }
    }

    impl std::error::Error for OriginalSourceError {}

    struct FailingReader;

    impl Read for FailingReader {
        fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other(OriginalSourceError))
        }
    }

    struct SwallowingSink;

    impl BundleSink for SwallowingSink {
        fn put_file<R: Read>(&mut self, _path: &str, reader: &mut R) -> Result<(), String> {
            let mut byte = [0];
            let _ = reader.read(&mut byte);
            Ok(())
        }
    }

    struct RejectingSink;

    impl BundleSink for RejectingSink {
        fn put_file<R: Read>(&mut self, _path: &str, reader: &mut R) -> Result<(), String> {
            let mut bytes = Vec::new();
            reader
                .read_to_end(&mut bytes)
                .expect("the test source should be readable");
            Err("target rejected the file".to_string())
        }
    }

    fn unverified_file() -> ArtifactFile {
        ArtifactFile {
            rel_path: "weights.bin".to_string(),
            size_bytes: None,
            checksum: None,
        }
    }

    #[test]
    fn preserves_a_source_error_even_when_the_sink_swallows_it() {
        let mut sink = SwallowingSink;

        let error = transfer_reader_to_sink(FailingReader, &mut sink, &unverified_file())
            .expect_err("the source failure must not be hidden by the sink");

        let DownloadError::Transfer { rel_path, source } = error else {
            panic!("unexpected error: {error:?}");
        };
        assert_eq!(rel_path, "weights.bin");
        assert_eq!(
            source.to_string(),
            "Transport error: original source failure"
        );
    }

    struct CancellingFailureReader {
        cancelled: Arc<AtomicBool>,
    }

    impl Read for CancellingFailureReader {
        fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
            self.cancelled.store(true, Ordering::SeqCst);
            Err(io::Error::other("source failed while cancellation arrived"))
        }
    }

    struct SharedCancellationObserver {
        cancelled: Arc<AtomicBool>,
    }

    impl TransferObserver for SharedCancellationObserver {
        fn is_cancelled(&self) -> bool {
            self.cancelled.load(Ordering::SeqCst)
        }
    }

    #[test]
    fn cancellation_takes_precedence_over_a_simultaneous_source_error() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let reader = CancellingFailureReader {
            cancelled: Arc::clone(&cancelled),
        };
        let mut observer = SharedCancellationObserver { cancelled };
        let mut sink = SwallowingSink;

        let error = transfer_reader_to_sink_with_observer(
            reader,
            &mut sink,
            &unverified_file(),
            &mut observer,
        )
        .expect_err("cancellation should stop the transfer");

        assert!(matches!(
            error,
            DownloadError::Cancelled { rel_path } if rel_path == "weights.bin"
        ));
    }

    #[test]
    fn target_error_takes_precedence_over_verification() {
        let file = ArtifactFile {
            rel_path: "weights.bin".to_string(),
            size_bytes: Some(999),
            checksum: Some("00".repeat(32)),
        };
        let mut sink = RejectingSink;

        let error = transfer_reader_to_sink(Cursor::new(b"payload"), &mut sink, &file)
            .expect_err("the target should reject the file before verification");

        assert!(matches!(
            error,
            DownloadError::TargetError(message) if message == "target rejected the file"
        ));
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

        fn get_reader(
            &self,
            _url: &str,
            _expected_size_bytes: Option<u64>,
        ) -> Result<Box<dyn Read + Send>, TransferError> {
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
