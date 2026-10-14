//! This module provides utilities for uploading artifact files from any source to any target bundle sink using multipart uploads with presigned URLs.
//!
//! The upload process can be customized with any implementation of the FileTransferClient trait (e.g. for custom HTTP clients, authentication, retries, etc), and multipart file sources can be abstracted behind the MultipartUploadSource trait for maximum flexibility (e.g. to support streaming from large files without loading them fully into memory).

use std::collections::HashSet;
use std::io::{self, Read};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use crossbeam::channel::{self, Receiver, Select, Sender};

use crate::transfer::{TransferError, TransferObserver};
use crate::{FileTransferClient, ReqwestTransferClient};

/// Errors that can occur during artifact file uploads.
#[derive(Debug, thiserror::Error)]
pub enum UploadError {
    /// The caller cancelled while a file was being uploaded.
    #[error("upload cancelled while transferring {rel_path}")]
    Cancelled { rel_path: String },
    /// Errors from the transfer client (e.g. network errors, HTTP errors).
    #[error("transfer error for part {part_index} of {total_parts} for {rel_path}: {source}")]
    Transfer {
        part_index: usize,
        total_parts: usize,
        rel_path: String,
        #[source]
        source: TransferError,
    },
    /// Errors related to invalid multipart upload plans (e.g. duplicate paths, invalid part numbering).
    #[error("invalid multipart upload plan: {0}")]
    InvalidMultipart(String),
    /// Errors related to multipart reader issues (e.g. file access errors).
    #[error("multipart reader error for {rel_path}: {source}")]
    MultipartReader {
        rel_path: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

/// One multipart upload part descriptor.
#[derive(Debug, Clone)]
pub struct MultipartUploadPart {
    pub part: u32,
    pub url: String,
    pub size_bytes: u64,
}

/// One file multipart upload descriptor.
#[derive(Debug, Clone)]
pub struct MultipartUploadFile {
    pub rel_path: String,
    pub parts: Vec<MultipartUploadPart>,
}

/// Source abstraction for multipart uploads.
pub trait MultipartUploadSource {
    /// Return the file length in bytes for a relative path.
    fn file_len(&self, rel_path: &str) -> Result<u64, UploadError>;

    /// Open a reader for one file chunk.
    fn open_part(
        &self,
        rel_path: &str,
        offset: u64,
        size: u64,
    ) -> Result<Box<dyn Read + Send>, UploadError>;
}

/// So a borrowed source can be handed to code that wants to own one.
impl<S: MultipartUploadSource + ?Sized> MultipartUploadSource for &S {
    fn file_len(&self, rel_path: &str) -> Result<u64, UploadError> {
        (**self).file_len(rel_path)
    }

    fn open_part(
        &self,
        rel_path: &str,
        offset: u64,
        size: u64,
    ) -> Result<Box<dyn Read + Send>, UploadError> {
        (**self).open_part(rel_path, offset, size)
    }
}

/// Upload multiple files from a multipart source using presigned URLs.
pub fn upload_bundle_multipart<S: MultipartUploadSource>(
    source: &S,
    files: &[MultipartUploadFile],
) -> Result<(), UploadError> {
    let client = ReqwestTransferClient::new();
    upload_bundle_multipart_with_client(&client, source, files)
}

/// Upload multiple files from a multipart source using presigned URLs and a custom client.
pub fn upload_bundle_multipart_with_client<FTC: FileTransferClient, S: MultipartUploadSource>(
    client: &FTC,
    source: &S,
    files: &[MultipartUploadFile],
) -> Result<(), UploadError> {
    upload_bundle_multipart_with_client_and_observer(client, source, files, &mut ())
}

/// Upload multiple files, reporting progress and honouring cancellation through `observer`.
///
/// The whole plan is checked against `source` before anything is sent. Parts are then uploaded
/// concurrently, up to eight at a time, in file and part order. `source` opens each
/// part and `observer` hears of each file on the calling thread, so several files can be in
/// progress at once. The first failure, or a cancellation, stops the parts not yet started and
/// aborts those in flight.
pub fn upload_bundle_multipart_with_client_and_observer<
    FTC: FileTransferClient,
    S: MultipartUploadSource,
    O: TransferObserver,
>(
    client: &FTC,
    source: &S,
    files: &[MultipartUploadFile],
    observer: &mut O,
) -> Result<(), UploadError> {
    let plan = plan_uploads(source, files)?;
    let workers = UPLOAD_WORKERS.min(plan.iter().map(|file| file.parts.len()).sum::<usize>());
    let stop = Arc::new(AtomicBool::new(false));
    let mut run = UploadRun::new(&plan, observer, Arc::clone(&stop));

    thread::scope(|scope| {
        let (jobs, queue) = channel::bounded::<PartJob<'_>>(0);
        let (events, reports) = channel::unbounded();
        for _ in 0..workers {
            let queue = queue.clone();
            let events = events.clone();
            let stop = Arc::clone(&stop);
            scope.spawn(move || {
                for job in queue {
                    let reader = CountingReader {
                        inner: job.reader,
                        file: job.file,
                        events: events.clone(),
                        stop: Arc::clone(&stop),
                    };
                    let result = client.put_reader(job.url, reader, job.size);
                    let _ = events.send(PartEvent::Finished {
                        file: job.file,
                        part: job.part,
                        result,
                    });
                }
            });
        }
        drop(queue);
        drop(events);

        'files: for (file, planned) in plan.iter().enumerate() {
            if !run.start(file) {
                break;
            }
            for part in &planned.parts {
                run.check_cancelled(file);
                if run.stopped() {
                    break 'files;
                }
                let reader = match source.open_part(planned.rel_path, part.offset, part.size) {
                    Ok(reader) => reader,
                    Err(error) => {
                        run.fail(error);
                        break 'files;
                    }
                };
                let job = PartJob {
                    file,
                    part: part.number,
                    url: part.url,
                    size: part.size,
                    reader,
                };
                if !run.hand_over(job, &jobs, &reports) {
                    break 'files;
                }
            }
        }
        drop(jobs);
        while run.in_flight() {
            match reports.recv() {
                Ok(event) => run.handle(event),
                Err(_) => break,
            }
        }
    });

    run.finish()
}

/// The most parts uploaded at once.
const UPLOAD_WORKERS: usize = 8;

/// A file of the plan, checked against its source.
struct PlannedFile<'a> {
    rel_path: &'a str,
    len: u64,
    parts: Vec<PlannedPart<'a>>,
}

/// A part of a planned file: its 1-based number, URL, and the bytes of the file it covers.
struct PlannedPart<'a> {
    number: usize,
    url: &'a str,
    offset: u64,
    size: u64,
}

/// Checks that no file is described twice and that each file's parts are numbered from 1 and
/// cover it exactly, in part order.
fn plan_uploads<'a, S: MultipartUploadSource>(
    source: &S,
    files: &'a [MultipartUploadFile],
) -> Result<Vec<PlannedFile<'a>>, UploadError> {
    let mut seen = HashSet::new();
    files
        .iter()
        .map(|file| {
            if !seen.insert(file.rel_path.as_str()) {
                return Err(UploadError::InvalidMultipart(format!(
                    "Duplicate multipart upload descriptor for {}",
                    file.rel_path
                )));
            }
            let len = source.file_len(&file.rel_path)?;
            let mut parts: Vec<&MultipartUploadPart> = file.parts.iter().collect();
            parts.sort_by_key(|part| part.part);

            let mut offset = 0u64;
            let mut planned = Vec::with_capacity(parts.len());
            for (index, part) in parts.into_iter().enumerate() {
                let number = index + 1;
                if part.part != number as u32 {
                    return Err(UploadError::InvalidMultipart(format!(
                        "Invalid part numbering for {}: expected {}, got {}",
                        file.rel_path, number, part.part
                    )));
                }
                if offset + part.size_bytes > len {
                    return Err(UploadError::InvalidMultipart(format!(
                        "Part {} exceeds file length for {}",
                        number, file.rel_path
                    )));
                }
                planned.push(PlannedPart {
                    number,
                    url: &part.url,
                    offset,
                    size: part.size_bytes,
                });
                offset += part.size_bytes;
            }
            if offset != len {
                return Err(UploadError::InvalidMultipart(format!(
                    "Multipart size mismatch for {} (parts cover {}, expected {})",
                    file.rel_path, offset, len
                )));
            }

            Ok(PlannedFile {
                rel_path: &file.rel_path,
                len,
                parts: planned,
            })
        })
        .collect()
}

/// A part handed to a worker, with its open reader.
struct PartJob<'a> {
    file: usize,
    part: usize,
    url: &'a str,
    size: u64,
    reader: Box<dyn Read + Send>,
}

/// What a worker reports to the calling thread.
enum PartEvent {
    /// Bytes of a part of `file` were read to be sent.
    Sent { file: usize, bytes: u64 },
    /// A part of `file` was uploaded, or failed.
    Finished {
        file: usize,
        part: usize,
        result: Result<(), TransferError>,
    },
}

/// A part's reader that reports the bytes it gives, and fails once the upload is stopped, which
/// aborts the request sending it.
struct CountingReader {
    inner: Box<dyn Read + Send>,
    file: usize,
    events: Sender<PartEvent>,
    stop: Arc<AtomicBool>,
}

impl Read for CountingReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.stop.load(Ordering::Relaxed) {
            return Err(io::Error::other("the upload was stopped"));
        }
        let read = self.inner.read(buf)?;
        if read > 0 {
            let _ = self.events.send(PartEvent::Sent {
                file: self.file,
                bytes: read as u64,
            });
        }
        Ok(read)
    }
}

/// The calling thread's view of an upload: what each file has sent, what remains, and the first
/// failure.
struct UploadRun<'p, 'a, O: TransferObserver> {
    plan: &'p [PlannedFile<'a>],
    observer: &'p mut O,
    stop: Arc<AtomicBool>,
    sent: Vec<u64>,
    parts_left: Vec<usize>,
    handed_over: usize,
    finished: usize,
    failure: Option<UploadError>,
}

impl<'p, 'a, O: TransferObserver> UploadRun<'p, 'a, O> {
    fn new(plan: &'p [PlannedFile<'a>], observer: &'p mut O, stop: Arc<AtomicBool>) -> Self {
        Self {
            plan,
            observer,
            stop,
            sent: vec![0; plan.len()],
            parts_left: plan.iter().map(|file| file.parts.len()).collect(),
            handed_over: 0,
            finished: 0,
            failure: None,
        }
    }

    /// Announces `file`, completing it at once when it has no parts. False once stopped.
    fn start(&mut self, file: usize) -> bool {
        self.check_cancelled(file);
        if self.stopped() {
            return false;
        }
        let planned = &self.plan[file];
        self.observer
            .file_started(planned.rel_path, Some(planned.len));
        if planned.parts.is_empty() {
            self.observer.file_completed(planned.rel_path, 0);
        }
        true
    }

    /// Hands `job` to a worker, handling reports while all are busy. False once stopped.
    fn hand_over(
        &mut self,
        job: PartJob<'a>,
        jobs: &Sender<PartJob<'a>>,
        reports: &Receiver<PartEvent>,
    ) -> bool {
        let mut job = Some(job);
        while let Some(pending) = job.take() {
            let mut select = Select::new();
            let send = select.send(jobs);
            let receive = select.recv(reports);
            let operation = select.select();
            match operation.index() {
                index if index == send => {
                    if operation.send(jobs, pending).is_err() {
                        return false;
                    }
                    self.handed_over += 1;
                }
                index if index == receive => {
                    job = Some(pending);
                    match operation.recv(reports) {
                        Ok(event) => self.handle(event),
                        Err(_) => return false,
                    }
                    if self.stopped() {
                        return false;
                    }
                }
                _ => unreachable!("only two operations are selected"),
            }
        }
        true
    }

    /// Whether a part handed to a worker has not finished yet.
    fn in_flight(&self) -> bool {
        self.finished < self.handed_over
    }

    /// Takes in a worker's report.
    fn handle(&mut self, event: PartEvent) {
        if let PartEvent::Finished { .. } = event {
            self.finished += 1;
        }
        let file = match event {
            PartEvent::Sent { file, bytes } => {
                self.sent[file] += bytes;
                if self.failure.is_none() {
                    self.observer
                        .file_progress(self.plan[file].rel_path, self.sent[file]);
                }
                file
            }
            PartEvent::Finished {
                file,
                result: Ok(()),
                ..
            } => {
                self.parts_left[file] -= 1;
                if self.parts_left[file] == 0 && self.failure.is_none() {
                    let planned = &self.plan[file];
                    self.observer.file_completed(planned.rel_path, planned.len);
                }
                file
            }
            PartEvent::Finished {
                file,
                part,
                result: Err(source),
            } => {
                let planned = &self.plan[file];
                self.fail(UploadError::Transfer {
                    part_index: part,
                    total_parts: planned.parts.len(),
                    rel_path: planned.rel_path.to_string(),
                    source,
                });
                file
            }
        };
        self.check_cancelled(file);
    }

    fn check_cancelled(&mut self, file: usize) {
        if self.failure.is_none() && self.observer.is_cancelled() {
            self.fail(UploadError::Cancelled {
                rel_path: self.plan[file].rel_path.to_string(),
            });
        }
    }

    /// Stops the upload on `error`, unless it already stopped on another.
    fn fail(&mut self, error: UploadError) {
        if self.failure.is_none() {
            self.failure = Some(error);
        }
        self.stop.store(true, Ordering::Relaxed);
    }

    fn stopped(&self) -> bool {
        self.failure.is_some()
    }

    fn finish(self) -> Result<(), UploadError> {
        match self.failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfer::TransferError;
    use std::collections::HashMap;
    use std::io::{Cursor, Read};
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct MockClient {
        #[allow(clippy::type_complexity)]
        puts: Arc<Mutex<Vec<(String, u64, Vec<u8>)>>>,
    }

    impl FileTransferClient for MockClient {
        fn put_reader<R: Read + Send + 'static>(
            &self,
            url: &str,
            mut reader: R,
            size_bytes: u64,
        ) -> Result<(), TransferError> {
            let mut bytes = Vec::new();
            reader
                .read_to_end(&mut bytes)
                .map_err(|e| TransferError::Transport(e.to_string()))?;
            self.puts
                .lock()
                .expect("lock puts")
                .push((url.to_string(), size_bytes, bytes));
            Ok(())
        }

        fn get_reader(
            &self,
            _url: &str,
            _expected_size_bytes: Option<u64>,
        ) -> Result<Box<dyn Read + Send>, TransferError> {
            Err(TransferError::Transport(
                "get_reader should not be used in upload tests".to_string(),
            ))
        }
    }

    struct MockSource {
        files: HashMap<String, Vec<u8>>,
    }

    impl MockSource {
        fn new(files: HashMap<String, Vec<u8>>) -> Self {
            Self { files }
        }
    }

    impl MultipartUploadSource for MockSource {
        fn file_len(&self, rel_path: &str) -> Result<u64, UploadError> {
            let bytes = self.files.get(rel_path).ok_or_else(|| {
                UploadError::InvalidMultipart(format!("missing file: {rel_path}"))
            })?;
            Ok(bytes.len() as u64)
        }

        fn open_part(
            &self,
            rel_path: &str,
            offset: u64,
            size: u64,
        ) -> Result<Box<dyn Read + Send>, UploadError> {
            let bytes = self.files.get(rel_path).ok_or_else(|| {
                UploadError::InvalidMultipart(format!("missing file: {rel_path}"))
            })?;
            let start = offset as usize;
            let end = (offset + size) as usize;
            let slice = bytes
                .get(start..end)
                .ok_or_else(|| UploadError::MultipartReader {
                    rel_path: rel_path.to_string(),
                    source: format!(
                        "invalid part range [{start}..{end}) for file of len {}",
                        bytes.len()
                    )
                    .into(),
                })?;
            Ok(Box::new(Cursor::new(slice.to_vec())))
        }
    }

    #[test]
    fn rejects_non_contiguous_part_numbering() {
        let client = MockClient::default();
        let source = MockSource::new(HashMap::from([(
            "weights.bin".to_string(),
            b"abcd".to_vec(),
        )]));
        let files = vec![MultipartUploadFile {
            rel_path: "weights.bin".to_string(),
            parts: vec![
                MultipartUploadPart {
                    part: 1,
                    url: "u1".to_string(),
                    size_bytes: 2,
                },
                MultipartUploadPart {
                    part: 3,
                    url: "u3".to_string(),
                    size_bytes: 2,
                },
            ],
        }];

        let err = upload_bundle_multipart_with_client(&client, &source, &files)
            .expect_err("part numbering must be contiguous");

        match err {
            UploadError::InvalidMultipart(msg) => assert!(msg.contains("expected 2, got 3")),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn rejects_part_plan_exceeding_file_len() {
        let client = MockClient::default();
        let source = MockSource::new(HashMap::from([(
            "weights.bin".to_string(),
            b"abc".to_vec(),
        )]));
        let files = vec![MultipartUploadFile {
            rel_path: "weights.bin".to_string(),
            parts: vec![
                MultipartUploadPart {
                    part: 1,
                    url: "u1".to_string(),
                    size_bytes: 2,
                },
                MultipartUploadPart {
                    part: 2,
                    url: "u2".to_string(),
                    size_bytes: 2,
                },
            ],
        }];

        let err = upload_bundle_multipart_with_client(&client, &source, &files)
            .expect_err("total part sizes cannot exceed file len");

        match err {
            UploadError::InvalidMultipart(msg) => assert!(msg.contains("exceeds file length")),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn uploads_all_parts_with_expected_content() {
        let client = MockClient::default();
        let source = MockSource::new(HashMap::from([(
            "weights.bin".to_string(),
            b"abcdef".to_vec(),
        )]));
        let files = vec![MultipartUploadFile {
            rel_path: "weights.bin".to_string(),
            parts: vec![
                MultipartUploadPart {
                    part: 1,
                    url: "u1".to_string(),
                    size_bytes: 2,
                },
                MultipartUploadPart {
                    part: 2,
                    url: "u2".to_string(),
                    size_bytes: 2,
                },
                MultipartUploadPart {
                    part: 3,
                    url: "u3".to_string(),
                    size_bytes: 2,
                },
            ],
        }];

        upload_bundle_multipart_with_client(&client, &source, &files)
            .expect("valid multipart plan should upload");

        let mut puts = client.puts.lock().expect("lock puts").clone();
        puts.sort();
        assert_eq!(
            puts,
            [
                ("u1".to_string(), 2, b"ab".to_vec()),
                ("u2".to_string(), 2, b"cd".to_vec()),
                ("u3".to_string(), 2, b"ef".to_vec()),
            ]
        );
    }

    fn two_part_file(rel_path: &str) -> MultipartUploadFile {
        MultipartUploadFile {
            rel_path: rel_path.to_string(),
            parts: vec![
                MultipartUploadPart {
                    part: 1,
                    url: format!("{rel_path}/1"),
                    size_bytes: 2,
                },
                MultipartUploadPart {
                    part: 2,
                    url: format!("{rel_path}/2"),
                    size_bytes: 2,
                },
            ],
        }
    }

    /// Holds each upload until another one is in flight, or a while has passed, and records the
    /// most uploads in flight at once.
    #[derive(Clone, Default)]
    struct OverlapClient {
        in_flight: Arc<Mutex<usize>>,
        most_in_flight: Arc<Mutex<usize>>,
    }

    impl FileTransferClient for OverlapClient {
        fn put_reader<R: Read + Send + 'static>(
            &self,
            _url: &str,
            mut reader: R,
            _size_bytes: u64,
        ) -> Result<(), TransferError> {
            let mut sink = Vec::new();
            reader.read_to_end(&mut sink).expect("read the part");
            *self.in_flight.lock().unwrap() += 1;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                let in_flight = *self.in_flight.lock().unwrap();
                let mut most = self.most_in_flight.lock().unwrap();
                *most = (*most).max(in_flight);
                if *most > 1 || std::time::Instant::now() > deadline {
                    break;
                }
                drop(most);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            *self.in_flight.lock().unwrap() -= 1;
            Ok(())
        }

        fn get_reader(
            &self,
            _url: &str,
            _expected_size_bytes: Option<u64>,
        ) -> Result<Box<dyn Read + Send>, TransferError> {
            unreachable!("uploads only")
        }
    }

    #[test]
    fn uploads_parts_concurrently() {
        let client = OverlapClient::default();
        let source = MockSource::new(HashMap::from([(
            "weights.bin".to_string(),
            b"abcd".to_vec(),
        )]));

        upload_bundle_multipart_with_client(&client, &source, &[two_part_file("weights.bin")])
            .expect("valid multipart plan should upload");

        assert!(*client.most_in_flight.lock().unwrap() > 1);
    }

    #[derive(Default)]
    struct RecordingObserver {
        started: Vec<(String, Option<u64>)>,
        progress: HashMap<String, u64>,
        completed: Vec<(String, u64)>,
        cancel: bool,
    }

    impl TransferObserver for RecordingObserver {
        fn is_cancelled(&self) -> bool {
            self.cancel
        }

        fn file_started(&mut self, rel_path: &str, total_bytes: Option<u64>) {
            self.started.push((rel_path.to_string(), total_bytes));
        }

        fn file_progress(&mut self, rel_path: &str, transferred_bytes: u64) {
            self.progress
                .insert(rel_path.to_string(), transferred_bytes);
        }

        fn file_completed(&mut self, rel_path: &str, transferred_bytes: u64) {
            self.completed
                .push((rel_path.to_string(), transferred_bytes));
        }
    }

    #[test]
    fn reports_each_file_from_start_to_completion() {
        let client = MockClient::default();
        let source = MockSource::new(HashMap::from([
            ("a.bin".to_string(), b"abcd".to_vec()),
            ("b.bin".to_string(), b"efgh".to_vec()),
        ]));
        let mut observer = RecordingObserver::default();

        upload_bundle_multipart_with_client_and_observer(
            &client,
            &source,
            &[two_part_file("a.bin"), two_part_file("b.bin")],
            &mut observer,
        )
        .expect("valid multipart plan should upload");

        let mut completed = observer.completed.clone();
        completed.sort();
        assert_eq!(
            completed,
            [("a.bin".to_string(), 4), ("b.bin".to_string(), 4)]
        );
        assert_eq!(
            observer.progress,
            HashMap::from([("a.bin".to_string(), 4), ("b.bin".to_string(), 4)])
        );
    }

    /// Fails the upload to one URL.
    #[derive(Clone)]
    struct FailingClient {
        failing_url: &'static str,
    }

    impl FileTransferClient for FailingClient {
        fn put_reader<R: Read + Send + 'static>(
            &self,
            url: &str,
            mut reader: R,
            _size_bytes: u64,
        ) -> Result<(), TransferError> {
            let mut sink = Vec::new();
            if reader.read_to_end(&mut sink).is_err() {
                return Err(TransferError::Transport("the part was aborted".to_string()));
            }
            if url == self.failing_url {
                return Err(TransferError::Transport("connection reset".to_string()));
            }
            Ok(())
        }

        fn get_reader(
            &self,
            _url: &str,
            _expected_size_bytes: Option<u64>,
        ) -> Result<Box<dyn Read + Send>, TransferError> {
            unreachable!("uploads only")
        }
    }

    #[test]
    fn a_failed_part_fails_the_upload_naming_the_part() {
        let client = FailingClient {
            failing_url: "weights.bin/2",
        };
        let source = MockSource::new(HashMap::from([(
            "weights.bin".to_string(),
            b"abcd".to_vec(),
        )]));

        let err =
            upload_bundle_multipart_with_client(&client, &source, &[two_part_file("weights.bin")])
                .expect_err("a failed part fails the upload");

        match err {
            UploadError::Transfer {
                part_index,
                total_parts,
                rel_path,
                ..
            } => assert_eq!(
                (part_index, total_parts, rel_path.as_str()),
                (2, 2, "weights.bin")
            ),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn a_cancelled_upload_sends_nothing() {
        let client = MockClient::default();
        let source = MockSource::new(HashMap::from([(
            "weights.bin".to_string(),
            b"abcd".to_vec(),
        )]));
        let mut observer = RecordingObserver {
            cancel: true,
            ..RecordingObserver::default()
        };

        let err = upload_bundle_multipart_with_client_and_observer(
            &client,
            &source,
            &[two_part_file("weights.bin")],
            &mut observer,
        )
        .expect_err("a cancelled upload fails");

        assert!(matches!(err, UploadError::Cancelled { .. }), "{err:?}");
        assert!(client.puts.lock().unwrap().is_empty());
    }
}
