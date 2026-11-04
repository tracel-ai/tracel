//! This module provides utilities for uploading artifact files from any source to any target bundle sink using multipart uploads with presigned URLs.
//!
//! The upload process can be customized with any implementation of the FileTransferClient trait (e.g. for custom HTTP clients, authentication, retries, etc), and multipart file sources can be abstracted behind the MultipartUploadSource trait for maximum flexibility (e.g. to support streaming from large files without loading them fully into memory).

use std::collections::{HashMap, HashSet, VecDeque};
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
    /// The store refused a part's URL, and the [`PartUrlSigner`] could not sign the upload's
    /// part URLs again.
    #[error(
        "part {part_index} of {total_parts} for {rel_path} was refused ({refusal}), and signing \
         its URLs again failed: {source}"
    )]
    SigningAgain {
        part_index: usize,
        total_parts: usize,
        rel_path: String,
        refusal: TransferError,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
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

/// Signs an upload's part URLs again when the store refuses the ones it was given, as S3 does to
/// every presigned URL once the credentials that signed it have expired.
///
/// Called on the thread that started the upload.
pub trait PartUrlSigner {
    /// Returns every file of the upload with each of its parts signed afresh, numbered and sized
    /// as when the upload started.
    fn sign_again(
        &self,
    ) -> Result<Vec<MultipartUploadFile>, Box<dyn std::error::Error + Send + Sync>>;
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
    upload(client, source, files, observer, None)
}

/// Upload multiple files as [`upload_bundle_multipart_with_client_and_observer`] does, asking
/// `signer` for fresh URLs when the store refuses a part's.
///
/// The refused part is sent again with the fresh URLs, and so are the parts not yet sent; parts
/// already uploaded are not sent again. A part refused with URLs that have been signed again
/// since it was sent is sent again with the fresh ones, so parts refused together share one
/// signing. An upload signs its URLs again at most three times in a row with no part uploaded
/// in between, so a long upload can outlive any number of them; a refusal after the third fails
/// it with [`TransferError::Refused`].
pub fn upload_bundle_multipart_with_signer<
    FTC: FileTransferClient,
    S: MultipartUploadSource,
    O: TransferObserver,
>(
    client: &FTC,
    source: &S,
    files: &[MultipartUploadFile],
    observer: &mut O,
    signer: &dyn PartUrlSigner,
) -> Result<(), UploadError> {
    upload(client, source, files, observer, Some(signer))
}

fn upload<FTC: FileTransferClient, S: MultipartUploadSource, O: TransferObserver>(
    client: &FTC,
    source: &S,
    files: &[MultipartUploadFile],
    observer: &mut O,
    signer: Option<&dyn PartUrlSigner>,
) -> Result<(), UploadError> {
    let plan = plan_uploads(source, files)?;
    let workers = UPLOAD_WORKERS.min(plan.iter().map(|file| file.parts.len()).sum::<usize>());
    let stop = Arc::new(AtomicBool::new(false));
    let mut run = UploadRun::new(plan, observer, signer, Arc::clone(&stop));

    thread::scope(|scope| {
        let (jobs, queue) = channel::bounded::<PartJob>(0);
        let (events, reports) = channel::unbounded();
        for _ in 0..workers {
            let queue = queue.clone();
            let events = events.clone();
            let stop = Arc::clone(&stop);
            scope.spawn(move || {
                for job in queue {
                    let reader = CountingReader {
                        inner: job.reader,
                        part: job.part,
                        signing: job.signing,
                        events: events.clone(),
                        stop: Arc::clone(&stop),
                    };
                    let result = client.put_reader(&job.url, reader, job.size);
                    let _ = events.send(PartEvent::Finished {
                        part: job.part,
                        signing: job.signing,
                        result,
                    });
                }
            });
        }
        drop(queue);
        drop(events);

        let channels = WorkerChannels {
            jobs: &jobs,
            reports: &reports,
        };
        run.send_every_part(source, &channels);
        run.wait_for_parts_in_flight(source, &channels);
        drop(jobs);
    });

    run.finish()
}

/// The most parts uploaded at once.
const UPLOAD_WORKERS: usize = 8;

/// The most times in a row an upload signs its part URLs again with no part uploaded in between.
const SIGNINGS_AGAIN_WITHOUT_PROGRESS: usize = 3;

/// A file of the plan, checked against its source.
struct PlannedFile<'a> {
    rel_path: &'a str,
    len: u64,
    parts: Vec<PlannedPart>,
}

/// A part of a planned file: its 1-based number, URL, and the bytes of the file it covers.
struct PlannedPart {
    number: usize,
    url: String,
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
                    url: part.url.clone(),
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

/// Gives the plan the URLs of `signed`, which must describe the same files with the same parts.
fn replace_part_urls(
    plan: &mut [PlannedFile<'_>],
    signed: Vec<MultipartUploadFile>,
) -> Result<(), UploadError> {
    if signed.len() != plan.len() {
        return Err(UploadError::InvalidMultipart(format!(
            "The parts signed again describe {} files, the upload {}",
            signed.len(),
            plan.len()
        )));
    }
    let mut signed: HashMap<String, Vec<MultipartUploadPart>> = signed
        .into_iter()
        .map(|file| (file.rel_path, file.parts))
        .collect();
    let mut urls = Vec::with_capacity(plan.len());
    for planned in plan.iter() {
        let mismatch = || {
            UploadError::InvalidMultipart(format!(
                "The parts signed again do not match the upload of {}",
                planned.rel_path
            ))
        };
        let mut parts = signed.remove(planned.rel_path).ok_or_else(mismatch)?;
        parts.sort_by_key(|part| part.part);
        let matches = parts.len() == planned.parts.len()
            && parts.iter().zip(&planned.parts).all(|(part, planned)| {
                part.part as usize == planned.number && part.size_bytes == planned.size
            });
        if !matches {
            return Err(mismatch());
        }
        urls.push(parts.into_iter().map(|part| part.url));
    }

    for (planned, urls) in plan.iter_mut().zip(urls) {
        for (part, url) in planned.parts.iter_mut().zip(urls) {
            part.url = url;
        }
    }
    Ok(())
}

/// A part of the plan: the index of its file, and its own index among that file's parts.
#[derive(Debug, Clone, Copy)]
struct PartId {
    file: usize,
    index: usize,
}

/// A part handed to a worker, with its open reader. `signing` counts the times the upload's URLs
/// had been signed again when `url` was taken from the plan.
struct PartJob {
    part: PartId,
    signing: usize,
    url: String,
    size: u64,
    reader: Box<dyn Read + Send>,
}

/// What a worker reports to the calling thread.
enum PartEvent {
    /// Bytes of `part`, sent with the URLs of `signing`, were read to be sent.
    Sent {
        part: PartId,
        signing: usize,
        bytes: u64,
    },
    /// `part`, sent with the URLs of `signing`, was uploaded, or failed.
    Finished {
        part: PartId,
        signing: usize,
        result: Result<(), TransferError>,
    },
}

/// A part's reader that reports the bytes it gives, and fails once the upload is stopped, which
/// aborts the request sending it.
struct CountingReader {
    inner: Box<dyn Read + Send>,
    part: PartId,
    signing: usize,
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
                part: self.part,
                signing: self.signing,
                bytes: read as u64,
            });
        }
        Ok(read)
    }
}

/// The calling thread's ends of the channels to and from the workers.
struct WorkerChannels<'c> {
    jobs: &'c Sender<PartJob>,
    reports: &'c Receiver<PartEvent>,
}

/// The bytes counted for the latest sending of a part, and the signing of the URLs it was sent
/// with.
#[derive(Default, Clone, Copy)]
struct PartProgress {
    signing: usize,
    bytes: u64,
}

/// The calling thread's view of an upload: what each file has sent, what remains, the parts to
/// send again, and the first failure.
struct UploadRun<'p, 'a, O: TransferObserver> {
    plan: Vec<PlannedFile<'a>>,
    observer: &'p mut O,
    signer: Option<&'p dyn PartUrlSigner>,
    stop: Arc<AtomicBool>,
    sent: Vec<u64>,
    progress: Vec<Vec<PartProgress>>,
    parts_left: Vec<usize>,
    signings_again: usize,
    signings_again_without_progress: usize,
    refused: VecDeque<PartId>,
    handed_over: usize,
    finished: usize,
    failure: Option<UploadError>,
}

impl<'p, 'a, O: TransferObserver> UploadRun<'p, 'a, O> {
    fn new(
        plan: Vec<PlannedFile<'a>>,
        observer: &'p mut O,
        signer: Option<&'p dyn PartUrlSigner>,
        stop: Arc<AtomicBool>,
    ) -> Self {
        Self {
            sent: vec![0; plan.len()],
            progress: plan
                .iter()
                .map(|file| vec![PartProgress::default(); file.parts.len()])
                .collect(),
            parts_left: plan.iter().map(|file| file.parts.len()).collect(),
            plan,
            observer,
            signer,
            stop,
            signings_again: 0,
            signings_again_without_progress: 0,
            refused: VecDeque::new(),
            handed_over: 0,
            finished: 0,
            failure: None,
        }
    }

    /// Hands every part of the plan to the workers in file and part order, sending refused parts
    /// again first. Returns once all are handed over, or the upload stopped.
    fn send_every_part<S: MultipartUploadSource>(
        &mut self,
        source: &S,
        channels: &WorkerChannels<'_>,
    ) {
        for file in 0..self.plan.len() {
            if !self.start(file) {
                return;
            }
            for index in 0..self.plan[file].parts.len() {
                self.check_cancelled(file);
                if !self.send_refused_parts_again(source, channels)
                    || !self.send(PartId { file, index }, source, channels)
                {
                    return;
                }
            }
        }
    }

    /// Handles reports until no part is in flight, sending refused parts again meanwhile.
    fn wait_for_parts_in_flight<S: MultipartUploadSource>(
        &mut self,
        source: &S,
        channels: &WorkerChannels<'_>,
    ) {
        loop {
            if !self.stopped()
                && let Some(part) = self.refused.pop_front()
            {
                self.send(part, source, channels);
                continue;
            }
            if !self.in_flight() {
                return;
            }
            match channels.reports.recv() {
                Ok(event) => self.handle(event),
                Err(_) => return,
            }
        }
    }

    /// False once stopped.
    fn send_refused_parts_again<S: MultipartUploadSource>(
        &mut self,
        source: &S,
        channels: &WorkerChannels<'_>,
    ) -> bool {
        while let Some(part) = self.refused.pop_front() {
            if !self.send(part, source, channels) {
                return false;
            }
        }
        !self.stopped()
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

    /// Opens `part` and hands it to a worker with the plan's current URL. False once stopped.
    fn send<S: MultipartUploadSource>(
        &mut self,
        part: PartId,
        source: &S,
        channels: &WorkerChannels<'_>,
    ) -> bool {
        if self.stopped() {
            return false;
        }
        let file = &self.plan[part.file];
        let planned = &file.parts[part.index];
        let reader = match source.open_part(file.rel_path, planned.offset, planned.size) {
            Ok(reader) => reader,
            Err(error) => {
                self.fail(error);
                return false;
            }
        };
        let job = self.job(part, reader);
        self.hand_over(job, channels)
    }

    /// A job sending `part` from `reader` with the plan's current URL, its bytes counted afresh.
    fn job(&mut self, part: PartId, reader: Box<dyn Read + Send>) -> PartJob {
        self.forget_sent_bytes(part);
        self.progress[part.file][part.index].signing = self.signings_again;
        let planned = &self.plan[part.file].parts[part.index];
        PartJob {
            part,
            signing: self.signings_again,
            url: planned.url.clone(),
            size: planned.size,
            reader,
        }
    }

    /// Hands `job` to a worker, handling reports while all are busy. False once stopped.
    fn hand_over(&mut self, job: PartJob, channels: &WorkerChannels<'_>) -> bool {
        let mut job = Some(job);
        while let Some(mut pending) = job.take() {
            let mut select = Select::new();
            let send = select.send(channels.jobs);
            let receive = select.recv(channels.reports);
            let operation = select.select();
            match operation.index() {
                index if index == send => {
                    if operation.send(channels.jobs, pending).is_err() {
                        return false;
                    }
                    self.handed_over += 1;
                }
                index if index == receive => {
                    match operation.recv(channels.reports) {
                        Ok(event) => self.handle(event),
                        Err(_) => return false,
                    }
                    if self.stopped() {
                        return false;
                    }
                    if pending.signing < self.signings_again {
                        pending = self.job(pending.part, pending.reader);
                    }
                    job = Some(pending);
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
        let file = match event {
            PartEvent::Sent {
                part,
                signing,
                bytes,
            } => {
                let progress = &mut self.progress[part.file][part.index];
                if progress.signing == signing {
                    progress.bytes += bytes;
                    self.sent[part.file] += bytes;
                    if self.failure.is_none() {
                        self.observer
                            .file_progress(self.plan[part.file].rel_path, self.sent[part.file]);
                    }
                }
                part.file
            }
            PartEvent::Finished {
                part,
                result: Ok(()),
                ..
            } => {
                self.finished += 1;
                self.signings_again_without_progress = 0;
                self.parts_left[part.file] -= 1;
                if self.parts_left[part.file] == 0 && self.failure.is_none() {
                    let planned = &self.plan[part.file];
                    self.observer.file_completed(planned.rel_path, planned.len);
                }
                part.file
            }
            PartEvent::Finished {
                part,
                signing,
                result: Err(refusal @ TransferError::Refused { .. }),
            } => {
                self.finished += 1;
                self.forget_sent_bytes(part);
                self.send_again_after_refusal(part, signing, refusal);
                part.file
            }
            PartEvent::Finished {
                part,
                result: Err(source),
                ..
            } => {
                self.finished += 1;
                self.fail(self.transfer_failure(part, source));
                part.file
            }
        };
        self.check_cancelled(file);
    }

    /// Queues `part` to be sent again, signing the URLs again first unless that was done since
    /// `part` was sent with the URLs of `signing`.
    fn send_again_after_refusal(&mut self, part: PartId, signing: usize, refusal: TransferError) {
        if self.stopped() {
            return;
        }
        if signing == self.signings_again
            && let Err(error) = self.sign_again(part, refusal)
        {
            self.fail(error);
            return;
        }
        self.refused.push_back(part);
    }

    /// Signs the upload's URLs again for the refusal of `part`, or fails with that refusal once
    /// they were signed again as often in a row as an upload may with no part uploaded.
    fn sign_again(&mut self, part: PartId, refusal: TransferError) -> Result<(), UploadError> {
        let signer = match self.signer {
            Some(signer)
                if self.signings_again_without_progress < SIGNINGS_AGAIN_WITHOUT_PROGRESS =>
            {
                signer
            }
            _ => return Err(self.transfer_failure(part, refusal)),
        };
        let signed = signer.sign_again().map_err(|source| {
            let planned = &self.plan[part.file];
            UploadError::SigningAgain {
                part_index: planned.parts[part.index].number,
                total_parts: planned.parts.len(),
                rel_path: planned.rel_path.to_string(),
                refusal,
                source,
            }
        })?;
        replace_part_urls(&mut self.plan, signed)?;
        self.signings_again += 1;
        self.signings_again_without_progress += 1;
        Ok(())
    }

    fn forget_sent_bytes(&mut self, part: PartId) {
        let progress = &mut self.progress[part.file][part.index];
        self.sent[part.file] -= progress.bytes;
        progress.bytes = 0;
    }

    fn transfer_failure(&self, part: PartId, source: TransferError) -> UploadError {
        let planned = &self.plan[part.file];
        UploadError::Transfer {
            part_index: planned.parts[part.index].number,
            total_parts: planned.parts.len(),
            rel_path: planned.rel_path.to_string(),
            source,
        }
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
        self.refused.clear();
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

    fn expired_token() -> TransferError {
        TransferError::Refused { status: 400 }
    }

    /// Signs every part of `files` again as `{url}?signed={n}`, `n` counting the signings.
    struct CountingSigner {
        files: Vec<MultipartUploadFile>,
        signings: std::cell::Cell<usize>,
    }

    impl CountingSigner {
        fn new(files: &[MultipartUploadFile]) -> Self {
            Self {
                files: files.to_vec(),
                signings: std::cell::Cell::new(0),
            }
        }
    }

    impl PartUrlSigner for CountingSigner {
        fn sign_again(
            &self,
        ) -> Result<Vec<MultipartUploadFile>, Box<dyn std::error::Error + Send + Sync>> {
            let signing = self.signings.get() + 1;
            self.signings.set(signing);
            Ok(self
                .files
                .iter()
                .map(|file| MultipartUploadFile {
                    rel_path: file.rel_path.clone(),
                    parts: file
                        .parts
                        .iter()
                        .map(|part| MultipartUploadPart {
                            url: format!("{}?signed={signing}", part.url),
                            ..part.clone()
                        })
                        .collect(),
                })
                .collect())
        }
    }

    type AcceptedParts = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

    /// Answers `refusal` to the URLs `refuses` picks, and records the parts it accepts.
    #[derive(Clone)]
    struct RefusingClient {
        refuses: fn(&str) -> bool,
        refusal: fn() -> TransferError,
        accepted: AcceptedParts,
    }

    impl RefusingClient {
        fn new(refuses: fn(&str) -> bool, refusal: fn() -> TransferError) -> Self {
            Self {
                refuses,
                refusal,
                accepted: Arc::default(),
            }
        }

        fn accepted(&self) -> Vec<(String, Vec<u8>)> {
            let mut accepted = self.accepted.lock().unwrap().clone();
            accepted.sort();
            accepted
        }
    }

    impl FileTransferClient for RefusingClient {
        fn put_reader<R: Read + Send + 'static>(
            &self,
            url: &str,
            mut reader: R,
            _size_bytes: u64,
        ) -> Result<(), TransferError> {
            let mut bytes = Vec::new();
            if reader.read_to_end(&mut bytes).is_err() {
                return Err(TransferError::Transport("the part was aborted".to_string()));
            }
            if (self.refuses)(url) {
                return Err((self.refusal)());
            }
            self.accepted.lock().unwrap().push((url.to_string(), bytes));
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
    fn a_refused_part_is_sent_again_with_its_urls_signed_again() {
        let client = RefusingClient::new(|url| url == "weights.bin/2", expired_token);
        let source = MockSource::new(HashMap::from([(
            "weights.bin".to_string(),
            b"abcd".to_vec(),
        )]));
        let files = [two_part_file("weights.bin")];
        let signer = CountingSigner::new(&files);
        let mut observer = RecordingObserver::default();

        upload_bundle_multipart_with_signer(&client, &source, &files, &mut observer, &signer)
            .expect("the refused part goes through once signed again");

        assert_eq!(signer.signings.get(), 1);
        assert_eq!(
            client.accepted(),
            [
                ("weights.bin/1".to_string(), b"ab".to_vec()),
                ("weights.bin/2?signed=1".to_string(), b"cd".to_vec()),
            ]
        );
        assert_eq!(
            observer.progress,
            HashMap::from([("weights.bin".to_string(), 4)])
        );
        assert_eq!(observer.completed, [("weights.bin".to_string(), 4)]);
    }

    #[test]
    fn a_url_refused_three_signings_in_a_row_fails_the_upload_with_the_refusal() {
        let client = RefusingClient::new(|_| true, || TransferError::Refused { status: 403 });
        let source = MockSource::new(HashMap::from([(
            "weights.bin".to_string(),
            b"abcd".to_vec(),
        )]));
        let files = [two_part_file("weights.bin")];
        let signer = CountingSigner::new(&files);

        let err = upload_bundle_multipart_with_signer(&client, &source, &files, &mut (), &signer)
            .expect_err("a URL refused however often it is signed fails the upload");

        assert_eq!(signer.signings.get(), SIGNINGS_AGAIN_WITHOUT_PROGRESS);
        assert!(
            matches!(
                &err,
                UploadError::Transfer {
                    source: TransferError::Refused { status: 403 },
                    ..
                }
            ),
            "{err:?}"
        );
    }

    #[test]
    fn a_transport_failure_is_not_signed_again() {
        let client = FailingClient {
            failing_url: "weights.bin/2",
        };
        let source = MockSource::new(HashMap::from([(
            "weights.bin".to_string(),
            b"abcd".to_vec(),
        )]));
        let files = [two_part_file("weights.bin")];
        let signer = CountingSigner::new(&files);

        let err = upload_bundle_multipart_with_signer(&client, &source, &files, &mut (), &signer)
            .expect_err("a failed part fails the upload");

        assert_eq!(signer.signings.get(), 0);
        assert!(
            matches!(
                err,
                UploadError::Transfer {
                    source: TransferError::Transport(_),
                    ..
                }
            ),
            "{err:?}"
        );
    }

    struct FailingSigner;

    impl PartUrlSigner for FailingSigner {
        fn sign_again(
            &self,
        ) -> Result<Vec<MultipartUploadFile>, Box<dyn std::error::Error + Send + Sync>> {
            Err("the version is no longer pending".into())
        }
    }

    #[test]
    fn a_failed_signing_fails_the_upload_naming_the_refusal() {
        let client = RefusingClient::new(|url| url == "weights.bin/2", expired_token);
        let source = MockSource::new(HashMap::from([(
            "weights.bin".to_string(),
            b"abcd".to_vec(),
        )]));

        let err = upload_bundle_multipart_with_signer(
            &client,
            &source,
            &[two_part_file("weights.bin")],
            &mut (),
            &FailingSigner,
        )
        .expect_err("a refused part that cannot be signed again fails the upload");

        match err {
            UploadError::SigningAgain {
                part_index,
                refusal: TransferError::Refused { status },
                ..
            } => assert_eq!((part_index, status), (2, 400)),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    /// Refuses every URL that was not signed again, holding each refusal until another one is in
    /// flight, or a while has passed, and records the most refused at once and the parts it
    /// accepts.
    #[derive(Clone, Default)]
    struct ExpiredUrlsClient {
        refusing: Arc<Mutex<usize>>,
        most_refusing: Arc<Mutex<usize>>,
        accepted: Arc<Mutex<Vec<String>>>,
    }

    impl FileTransferClient for ExpiredUrlsClient {
        fn put_reader<R: Read + Send + 'static>(
            &self,
            url: &str,
            mut reader: R,
            _size_bytes: u64,
        ) -> Result<(), TransferError> {
            let mut sink = Vec::new();
            reader.read_to_end(&mut sink).expect("read the part");
            if url.contains("?signed=") {
                self.accepted.lock().unwrap().push(url.to_string());
                return Ok(());
            }
            *self.refusing.lock().unwrap() += 1;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                let refusing = *self.refusing.lock().unwrap();
                let mut most = self.most_refusing.lock().unwrap();
                *most = (*most).max(refusing);
                if *most > 1 || std::time::Instant::now() > deadline {
                    break;
                }
                drop(most);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            *self.refusing.lock().unwrap() -= 1;
            Err(expired_token())
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
    fn parts_refused_together_share_one_signing() {
        let client = ExpiredUrlsClient::default();
        let source = MockSource::new(HashMap::from([(
            "weights.bin".to_string(),
            b"abcd".to_vec(),
        )]));
        let files = [two_part_file("weights.bin")];
        let signer = CountingSigner::new(&files);

        upload_bundle_multipart_with_signer(&client, &source, &files, &mut (), &signer)
            .expect("both parts go through once signed again");

        assert_eq!(*client.most_refusing.lock().unwrap(), 2);
        assert_eq!(signer.signings.get(), 1);
        let mut accepted = client.accepted.lock().unwrap().clone();
        accepted.sort();
        assert_eq!(
            accepted,
            ["weights.bin/1?signed=1", "weights.bin/2?signed=1"]
        );
    }

    const PROGRESSING_FILES: usize = 5;

    fn one_part_file(number: usize) -> MultipartUploadFile {
        MultipartUploadFile {
            rel_path: format!("{number}.bin"),
            parts: vec![MultipartUploadPart {
                part: 1,
                url: format!("{number}.bin/1"),
                size_bytes: 2,
            }],
        }
    }

    /// Records the files completed, for [`ProgressingClient`] to wait on.
    struct CompletionObserver {
        completed: Arc<Mutex<HashSet<String>>>,
    }

    impl TransferObserver for CompletionObserver {
        fn file_completed(&mut self, rel_path: &str, _transferred_bytes: u64) {
            self.completed.lock().unwrap().insert(rel_path.to_string());
        }
    }

    /// Accepts file `n` once its URL has been signed again `n - 1` times, so each signing lets one
    /// more file through. A refusal of URLs signed again `s` times waits for file `s + 1`, which
    /// those URLs let through, to be completed first, so a part is always uploaded between two
    /// signings.
    #[derive(Clone)]
    struct ProgressingClient {
        completed: Arc<Mutex<HashSet<String>>>,
        accepted: Arc<Mutex<Vec<String>>>,
    }

    impl FileTransferClient for ProgressingClient {
        fn put_reader<R: Read + Send + 'static>(
            &self,
            url: &str,
            mut reader: R,
            _size_bytes: u64,
        ) -> Result<(), TransferError> {
            let mut sink = Vec::new();
            reader.read_to_end(&mut sink).expect("read the part");
            let file: usize = url.split('.').next().unwrap().parse().unwrap();
            let signing: usize = url
                .split_once("?signed=")
                .map_or(0, |(_, signing)| signing.parse().unwrap());
            if signing + 1 >= file {
                self.accepted.lock().unwrap().push(url.to_string());
                return Ok(());
            }
            if signing > 0 {
                let let_through = format!("{}.bin", signing + 1);
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                while !self.completed.lock().unwrap().contains(&let_through)
                    && std::time::Instant::now() < deadline
                {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            }
            Err(expired_token())
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
    fn an_upload_that_progresses_between_refusals_signs_again_as_often_as_it_needs() {
        let completed = Arc::new(Mutex::new(HashSet::new()));
        let client = ProgressingClient {
            completed: Arc::clone(&completed),
            accepted: Arc::default(),
        };
        let files: Vec<MultipartUploadFile> = (1..=PROGRESSING_FILES).map(one_part_file).collect();
        let source = MockSource::new(
            files
                .iter()
                .map(|file| (file.rel_path.clone(), b"ab".to_vec()))
                .collect(),
        );
        let signer = CountingSigner::new(&files);
        let mut observer = CompletionObserver { completed };

        upload_bundle_multipart_with_signer(&client, &source, &files, &mut observer, &signer)
            .expect("an upload that keeps progressing keeps signing its URLs again");

        assert_eq!(signer.signings.get(), PROGRESSING_FILES - 1);
        assert!(signer.signings.get() > SIGNINGS_AGAIN_WITHOUT_PROGRESS);
        assert_eq!(observer.completed.lock().unwrap().len(), PROGRESSING_FILES);
        assert_eq!(client.accepted.lock().unwrap().len(), PROGRESSING_FILES);
    }
}
