//! Reading one file through several concurrent ranged requests.
//!
//! The bytes come back in order through a plain [`Read`], so everything downstream — bundle
//! sinks, checksum verification, progress and cancellation — is unchanged by the split.

use std::collections::{BTreeMap, VecDeque};
use std::io::{self, Read};
use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use super::TransferError;

pub(crate) trait RangeSource: Clone + Send + Sync + 'static {
    fn get_whole_reader(&self, url: &str) -> Result<Box<dyn Read + Send>, TransferError>;

    fn get_range(
        &self,
        url: &str,
        offset: u64,
        length: u64,
    ) -> Result<RangeResponse, RangeSourceError>;
}

pub(crate) struct RangeResponse {
    pub reader: Box<dyn Read + Send>,
    pub range: Range<u64>,
    pub total_size: u64,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum RangeSourceError {
    #[error("the source does not serve ranges")]
    Unsupported,
    #[error(transparent)]
    Transfer(#[from] TransferError),
}

/// How a file is split across concurrent ranged requests.
///
/// At most `workers` ranges are scheduled ahead of the reader, so together with the range being
/// read a plan holds up to `(workers + 1) * range_bytes` in memory.
#[derive(Debug, Clone)]
struct RangePlan {
    /// Ranges fetched at once.
    workers: NonZeroUsize,
    /// Bytes one range asks for.
    range_bytes: NonZeroU64,
    /// Attempts a range gets before the read fails.
    attempts: NonZeroU32,
    /// Wait before retrying a range, multiplied by the attempt already spent.
    pub backoff: Duration,
}

impl Default for RangePlan {
    fn default() -> Self {
        Self {
            workers: NonZeroUsize::new(6).expect("default worker count is nonzero"),
            range_bytes: NonZeroU64::new(8 * 1024 * 1024).expect("default range size is nonzero"),
            attempts: NonZeroU32::new(3).expect("default attempt count is nonzero"),
            backoff: Duration::from_millis(400),
        }
    }
}

/// Opens `url` for reading under the default [`RangePlan`].
pub(crate) fn open_for_download<C: RangeSource>(
    client: &C,
    url: &str,
    size_bytes: Option<u64>,
) -> Result<Box<dyn Read + Send>, TransferError> {
    open_for_download_with_plan(client, url, size_bytes, &RangePlan::default())
}

/// Opens `url` for reading, splitting it across concurrent ranged requests when its announced
/// size makes that worth doing and the source serves ranges.
///
/// Falls back to one whole-file request when the size is unknown, when the file fits in a single
/// range, or when the source answers a ranged request with the whole file.
fn open_for_download_with_plan<C: RangeSource>(
    client: &C,
    url: &str,
    size_bytes: Option<u64>,
    plan: &RangePlan,
) -> Result<Box<dyn Read + Send>, TransferError> {
    let split = size_bytes.filter(|size| *size > plan.range_bytes.get());
    let Some(size_bytes) = split else {
        return client.get_whole_reader(url);
    };

    // The first range is the file's own first chunk, and fetching it here answers whether the
    // source serves ranges before any worker is spawned.
    let stop = Arc::new(AtomicBool::new(false));
    match request_range(
        client,
        url,
        0,
        plan.range_bytes.get(),
        size_bytes,
        plan,
        &stop,
    ) {
        Ok((first, attempts)) => Ok(Box::new(RangedReader::new(
            client, url, size_bytes, plan, first, attempts, stop,
        ))),
        Err(RangeSourceError::Unsupported) => client.get_whole_reader(url),
        Err(RangeSourceError::Transfer(error)) => Err(error),
    }
}

/// Reads a file's ranges as they arrive and hands the bytes on in order.
struct RangedReader<C> {
    ranges: usize,
    next: usize,
    current: Box<dyn Read + Send>,
    arrived: BTreeMap<usize, Vec<u8>>,
    results: Receiver<Result<(usize, Vec<u8>), RangeSourceError>>,
    jobs: Arc<RangeJobs>,
    next_to_schedule: usize,
    stop: Arc<AtomicBool>,
    workers: Option<WorkerStart<C>>,
}

struct WorkerStart<C> {
    client: C,
    url: String,
    size_bytes: u64,
    plan: RangePlan,
    count: usize,
    sender: SyncSender<Result<(usize, Vec<u8>), RangeSourceError>>,
}

impl<C: RangeSource> RangedReader<C> {
    fn new(
        client: &C,
        url: &str,
        size_bytes: u64,
        plan: &RangePlan,
        first: RangeResponse,
        first_attempts: u32,
        stop: Arc<AtomicBool>,
    ) -> Self {
        let ranges = size_bytes.div_ceil(plan.range_bytes.get()) as usize;
        let scheduled = plan.workers.get().min(ranges - 1);
        let jobs = Arc::new(RangeJobs::new(1..=scheduled));
        let (sender, results) = sync_channel(plan.workers.get());

        Self {
            ranges,
            next: 1,
            current: Box::new(RangeStream::new(
                client,
                url,
                size_bytes,
                plan,
                first,
                first_attempts,
                Arc::clone(&stop),
            )),
            arrived: BTreeMap::new(),
            results,
            jobs,
            next_to_schedule: scheduled + 1,
            stop,
            workers: Some(WorkerStart {
                client: client.clone(),
                url: url.to_string(),
                size_bytes,
                plan: plan.clone(),
                count: scheduled,
                sender,
            }),
        }
    }

    fn start_workers(&mut self) {
        let Some(start) = self.workers.take() else {
            return;
        };
        for _ in 0..start.count {
            let client = start.client.clone();
            let url = start.url.clone();
            let plan = start.plan.clone();
            let jobs = Arc::clone(&self.jobs);
            let stop = Arc::clone(&self.stop);
            let sender = start.sender.clone();
            let size_bytes = start.size_bytes;
            thread::spawn(move || {
                fetch_ranges(&client, &url, size_bytes, &plan, &jobs, &stop, &sender)
            });
        }
        // The reader's own handle would otherwise keep it from ever disconnecting.
        drop(start.sender);
    }

    fn schedule_next(&mut self) {
        if self.next_to_schedule < self.ranges {
            self.jobs.push(self.next_to_schedule);
            self.next_to_schedule += 1;
        }
    }

    /// The bytes of one range, waiting for it when it has not arrived yet.
    fn take(&mut self, index: usize) -> io::Result<Vec<u8>> {
        if let Some(bytes) = self.arrived.remove(&index) {
            self.schedule_next();
            return Ok(bytes);
        }
        loop {
            match self.results.recv() {
                Ok(Ok((arrived, bytes))) if arrived == index => {
                    self.schedule_next();
                    return Ok(bytes);
                }
                Ok(Ok((arrived, bytes))) => {
                    self.arrived.insert(arrived, bytes);
                }
                Ok(Err(error)) => return Err(io::Error::other(error)),
                Err(_) => {
                    return Err(io::Error::other(format!(
                        "the source stopped before range {index} arrived"
                    )));
                }
            }
        }
    }
}

impl<C: RangeSource> Read for RangedReader<C> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        self.start_workers();
        loop {
            let read = self.current.read(buf)?;
            if read > 0 || buf.is_empty() {
                return Ok(read);
            }
            if self.next >= self.ranges {
                return Ok(0);
            }
            // Release the exhausted range before waiting for another one so it does not count
            // against the reader's memory bound.
            self.current = Box::new(io::empty());
            let bytes = self.take(self.next)?;
            self.next += 1;
            self.current = Box::new(io::Cursor::new(bytes));
        }
    }
}

/// Streams the leading range directly to the caller, resuming the unread suffix when its body is
/// interrupted. Later ranges can be buffered by workers because the caller has already had an
/// opportunity to observe progress and cancellation by then.
struct RangeStream<C> {
    client: C,
    url: String,
    expected_size: u64,
    plan: RangePlan,
    reader: Box<dyn Read + Send>,
    next_offset: u64,
    end_offset: u64,
    spent: u32,
    stop: Arc<AtomicBool>,
}

impl<C: RangeSource> RangeStream<C> {
    fn new(
        client: &C,
        url: &str,
        expected_size: u64,
        plan: &RangePlan,
        response: RangeResponse,
        spent: u32,
        stop: Arc<AtomicBool>,
    ) -> Self {
        Self {
            client: client.clone(),
            url: url.to_string(),
            expected_size,
            plan: plan.clone(),
            reader: response.reader,
            next_offset: response.range.start,
            end_offset: response.range.end,
            spent,
            stop,
        }
    }

    fn resume(&mut self, mut error: RangeSourceError) -> io::Result<()> {
        loop {
            if self.spent >= self.plan.attempts.get() || self.stop.load(Ordering::Relaxed) {
                return Err(io::Error::other(error));
            }
            thread::sleep(self.plan.backoff * self.spent);
            self.spent += 1;
            match request_range_once(
                &self.client,
                &self.url,
                self.next_offset,
                self.end_offset - self.next_offset,
                self.expected_size,
            ) {
                Ok(response) => {
                    self.reader = response.reader;
                    return Ok(());
                }
                Err(next) => error = next,
            }
        }
    }
}

impl<C: RangeSource> Read for RangeStream<C> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        loop {
            if self.next_offset == self.end_offset {
                let mut extra = [0];
                return match self.reader.read(&mut extra) {
                    Ok(0) => Ok(0),
                    Ok(_) => Err(io::Error::other(format!(
                        "range ending at {} answered more bytes than requested",
                        self.end_offset
                    ))),
                    Err(error) => Err(error),
                };
            }

            let remaining = (self.end_offset - self.next_offset) as usize;
            let limit = buffer.len().min(remaining);
            match self.reader.read(&mut buffer[..limit]) {
                Ok(0) => self.resume(
                    TransferError::Transport(format!(
                        "range at {} ended before {}",
                        self.next_offset, self.end_offset
                    ))
                    .into(),
                )?,
                Ok(read) => {
                    self.next_offset += read as u64;
                    return Ok(read);
                }
                Err(error) => self.resume(TransferError::Transport(error.to_string()).into())?,
            }
        }
    }
}

impl<C> Drop for RangedReader<C> {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.jobs.wake_all();
    }
}

struct RangeJobs {
    pending: Mutex<VecDeque<usize>>,
    ready: Condvar,
}

impl RangeJobs {
    fn new(pending: impl IntoIterator<Item = usize>) -> Self {
        Self {
            pending: Mutex::new(pending.into_iter().collect()),
            ready: Condvar::new(),
        }
    }

    fn push(&self, index: usize) {
        self.pending
            .lock()
            .expect("lock range jobs")
            .push_back(index);
        self.ready.notify_one();
    }

    fn take(&self, stop: &AtomicBool) -> Option<usize> {
        let mut pending = self.pending.lock().expect("lock range jobs");
        loop {
            if stop.load(Ordering::Relaxed) {
                return None;
            }
            if let Some(index) = pending.pop_front() {
                return Some(index);
            }
            pending = self.ready.wait(pending).expect("wait for range job");
        }
    }

    fn wake_all(&self) {
        self.ready.notify_all();
    }
}

/// Claims ranges until they run out, the reader goes away, or one of them cannot be read.
fn fetch_ranges<C: RangeSource>(
    client: &C,
    url: &str,
    size_bytes: u64,
    plan: &RangePlan,
    jobs: &RangeJobs,
    stop: &AtomicBool,
    sender: &SyncSender<Result<(usize, Vec<u8>), RangeSourceError>>,
) {
    while let Some(index) = jobs.take(stop) {
        let offset = index as u64 * plan.range_bytes.get();
        debug_assert!(offset < size_bytes);

        let length = plan.range_bytes.get().min(size_bytes - offset);
        let result = read_range(client, url, offset, length, size_bytes, plan, stop);
        let failed = result.is_err();
        if sender.send(result.map(|bytes| (index, bytes))).is_err() || failed {
            if failed {
                stop.store(true, Ordering::Relaxed);
                jobs.wake_all();
            }
            return;
        }
    }
}

/// One range, retried while the failure is not the source refusing to serve ranges at all.
fn read_range<C: RangeSource>(
    client: &C,
    url: &str,
    offset: u64,
    length: u64,
    expected_size: u64,
    plan: &RangePlan,
    stop: &AtomicBool,
) -> Result<Vec<u8>, RangeSourceError> {
    let mut spent = 0;
    loop {
        let attempt = read_range_once(client, url, offset, length, expected_size);
        spent += 1;
        match attempt {
            Ok(bytes) => return Ok(bytes),
            Err(RangeSourceError::Unsupported) => return Err(RangeSourceError::Unsupported),
            Err(error) if spent >= plan.attempts.get() || stop.load(Ordering::Relaxed) => {
                return Err(error);
            }
            Err(_) => thread::sleep(plan.backoff * spent),
        }
    }
}

fn request_range<C: RangeSource>(
    client: &C,
    url: &str,
    offset: u64,
    length: u64,
    expected_size: u64,
    plan: &RangePlan,
    stop: &AtomicBool,
) -> Result<(RangeResponse, u32), RangeSourceError> {
    let mut spent = 0;
    loop {
        let attempt = request_range_once(client, url, offset, length, expected_size);
        spent += 1;
        match attempt {
            Ok(response) => return Ok((response, spent)),
            Err(RangeSourceError::Unsupported) => return Err(RangeSourceError::Unsupported),
            Err(error) if spent >= plan.attempts.get() || stop.load(Ordering::Relaxed) => {
                return Err(error);
            }
            Err(_) => thread::sleep(plan.backoff * spent),
        }
    }
}

fn read_range_once<C: RangeSource>(
    client: &C,
    url: &str,
    offset: u64,
    length: u64,
    expected_size: u64,
) -> Result<Vec<u8>, RangeSourceError> {
    let response = request_range_once(client, url, offset, length, expected_size)?;
    let mut bytes = Vec::with_capacity(length as usize);
    response
        .reader
        .take(length.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| TransferError::Transport(error.to_string()))?;

    // A short or long body would silently change the file, and the checksum would be blamed for
    // what is really a broken range response.
    if bytes.len() as u64 != length {
        return Err(TransferError::Transport(format!(
            "range at {offset} answered {} of {length} bytes",
            bytes.len()
        ))
        .into());
    }
    Ok(bytes)
}

fn request_range_once<C: RangeSource>(
    client: &C,
    url: &str,
    offset: u64,
    length: u64,
    expected_size: u64,
) -> Result<RangeResponse, RangeSourceError> {
    let response = client.get_range(url, offset, length)?;
    let expected_range = offset..offset + length;
    if response.range != expected_range {
        return Err(TransferError::Transport(format!(
            "requested range {expected_range:?}, source answered {:?}",
            response.range
        ))
        .into());
    }
    if response.total_size != expected_size {
        return Err(TransferError::Transport(format!(
            "source size is {} bytes, expected {expected_size} bytes",
            response.total_size
        ))
        .into());
    }

    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::atomic::AtomicUsize;
    use std::time::Instant;

    /// What a source does when asked for part of a file.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Ranges {
        Served,
        /// Answers the whole file however little was asked for.
        Ignored,
        /// Answers one byte short.
        Short,
        /// Never answers this range.
        Failing(usize),
        /// Announces a total different from the source length.
        WrongTotal(u64),
    }

    #[derive(Clone)]
    struct Source {
        bytes: Arc<Vec<u8>>,
        ranges: Ranges,
        range_calls: Arc<AtomicUsize>,
        whole_calls: Arc<AtomicUsize>,
        first_body_reads: Arc<AtomicUsize>,
        /// Makes later ranges arrive first, so ordering is actually exercised.
        stagger: Duration,
    }

    impl Source {
        fn new(len: usize, ranges: Ranges) -> Self {
            Self {
                bytes: Arc::new((0..len).map(|byte| byte as u8).collect()),
                ranges,
                range_calls: Arc::new(AtomicUsize::new(0)),
                whole_calls: Arc::new(AtomicUsize::new(0)),
                first_body_reads: Arc::new(AtomicUsize::new(0)),
                stagger: Duration::ZERO,
            }
        }

        fn staggered(mut self, stagger: Duration) -> Self {
            self.stagger = stagger;
            self
        }
    }

    impl RangeSource for Source {
        fn get_whole_reader(&self, _url: &str) -> Result<Box<dyn Read + Send>, TransferError> {
            self.whole_calls.fetch_add(1, Ordering::Relaxed);
            Ok(Box::new(Cursor::new(self.bytes.as_ref().clone())))
        }

        fn get_range(
            &self,
            _url: &str,
            offset: u64,
            length: u64,
        ) -> Result<RangeResponse, RangeSourceError> {
            let index = self.range_calls.fetch_add(1, Ordering::Relaxed);
            if !self.stagger.is_zero() {
                thread::sleep(self.stagger * (8 - (index as u32).min(7)));
            }
            match self.ranges {
                Ranges::Ignored => Err(RangeSourceError::Unsupported),
                Ranges::Failing(at) if at as u64 == offset => {
                    Err(TransferError::Transport("nope".into()).into())
                }
                served => {
                    let start = offset as usize;
                    let end = (start + length as usize).min(self.bytes.len());
                    let end = if served == Ranges::Short {
                        end - 1
                    } else {
                        end
                    };
                    let total_size = match served {
                        Ranges::WrongTotal(total) => total,
                        _ => self.bytes.len() as u64,
                    };
                    let reader: Box<dyn Read + Send> = if offset == 0 {
                        Box::new(CountingReader {
                            inner: Cursor::new(self.bytes[start..end].to_vec()),
                            reads: Arc::clone(&self.first_body_reads),
                        })
                    } else {
                        Box::new(Cursor::new(self.bytes[start..end].to_vec()))
                    };
                    Ok(RangeResponse {
                        reader,
                        range: offset..offset + length,
                        total_size,
                    })
                }
            }
        }
    }

    struct CountingReader {
        inner: Cursor<Vec<u8>>,
        reads: Arc<AtomicUsize>,
    }

    impl Read for CountingReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            self.reads.fetch_add(1, Ordering::Relaxed);
            self.inner.read(buffer)
        }
    }

    fn plan() -> RangePlan {
        RangePlan {
            workers: NonZeroUsize::new(4).unwrap(),
            range_bytes: NonZeroU64::new(1024).unwrap(),
            attempts: NonZeroU32::new(2).unwrap(),
            backoff: Duration::from_millis(1),
        }
    }

    fn read_all(source: &Source, size: Option<u64>) -> io::Result<Vec<u8>> {
        let mut reader =
            open_for_download_with_plan(source, "url", size, &plan()).map_err(io::Error::other)?;
        let mut read = Vec::new();
        reader.read_to_end(&mut read)?;
        Ok(read)
    }

    #[test]
    fn ranges_arriving_out_of_order_are_read_in_order() {
        let source =
            Source::new(8 * 1024 + 500, Ranges::Served).staggered(Duration::from_millis(2));
        let size = source.bytes.len() as u64;

        let read = read_all(&source, Some(size)).expect("every range is served");

        assert_eq!(read, *source.bytes);
        assert_eq!(source.whole_calls.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn fetching_stays_within_the_reader_window() {
        let source = Source::new(20 * 1024, Ranges::Served);
        let size = source.bytes.len() as u64;

        let mut reader = open_for_download_with_plan(&source, "url", Some(size), &plan())
            .expect("the first range is served");
        let mut first_byte = [0];
        reader
            .read_exact(&mut first_byte)
            .expect("start the workers");
        let expected = 1 + plan().workers.get();
        let deadline = Instant::now() + Duration::from_secs(1);
        while source.range_calls.load(Ordering::Relaxed) < expected {
            assert!(Instant::now() < deadline, "workers did not finish in time");
            thread::yield_now();
        }
        // Once the active window is full, workers must remain idle until the reader advances.
        thread::sleep(Duration::from_millis(20));

        assert_eq!(source.range_calls.load(Ordering::Relaxed), expected);
        drop(reader);
    }

    #[test]
    fn opening_does_not_read_a_body_or_start_workers() {
        let source = Source::new(8 * 1024, Ranges::Served);
        let size = source.bytes.len() as u64;

        let reader = open_for_download_with_plan(&source, "url", Some(size), &plan())
            .expect("the first range headers are served");

        assert_eq!(source.range_calls.load(Ordering::Relaxed), 1);
        assert_eq!(source.first_body_reads.load(Ordering::Relaxed), 0);
        drop(reader);
    }

    #[test]
    fn a_source_that_ignores_ranges_is_read_whole() {
        let source = Source::new(8 * 1024, Ranges::Ignored);
        let size = source.bytes.len() as u64;

        let read = read_all(&source, Some(size)).expect("the whole file is served");

        assert_eq!(read, *source.bytes);
        assert_eq!(source.whole_calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_file_within_one_range_is_not_split() {
        let source = Source::new(512, Ranges::Served);
        let size = source.bytes.len() as u64;

        let read = read_all(&source, Some(size)).expect("the whole file is served");

        assert_eq!(read, *source.bytes);
        assert_eq!(source.range_calls.load(Ordering::Relaxed), 0);
        assert_eq!(source.whole_calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn an_unannounced_size_is_read_whole() {
        let source = Source::new(8 * 1024, Ranges::Served);

        let read = read_all(&source, None).expect("the whole file is served");

        assert_eq!(read, *source.bytes);
        assert_eq!(source.range_calls.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn a_range_that_never_arrives_fails_the_read() {
        let source = Source::new(8 * 1024, Ranges::Failing(3 * 1024));
        let size = source.bytes.len() as u64;

        let error = read_all(&source, Some(size)).expect_err("the third range never arrives");

        assert!(error.to_string().contains("nope"), "{error}");
    }

    #[test]
    fn a_short_range_is_refused_rather_than_truncating_the_file() {
        let source = Source::new(8 * 1024, Ranges::Short);
        let size = source.bytes.len() as u64;

        let error = read_all(&source, Some(size)).expect_err("a short range is not the file");

        assert!(error.to_string().contains("ended before 1024"), "{error}");
    }

    #[test]
    fn a_source_with_a_different_total_size_is_refused() {
        let source = Source::new(8 * 1024, Ranges::WrongTotal(9 * 1024));
        let size = source.bytes.len() as u64;

        let error = read_all(&source, Some(size)).expect_err("the total size changed");

        assert!(
            error.to_string().contains("source size is 9216 bytes"),
            "{error}"
        );
    }
}
