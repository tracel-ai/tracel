//! Reading one file through several concurrent ranged requests.
//!
//! The bytes come back in order through a plain [`Read`], so everything downstream — bundle
//! sinks, checksum verification, progress and cancellation — is unchanged by the split.

use std::collections::BTreeMap;
use std::io::{self, Read};
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use crossbeam::channel::{Receiver, Sender, bounded};

use super::{ReqwestTransferClient, TransferError};

trait RangeSource: Clone + Send + Sync + 'static {
    fn get_whole_reader(&self, url: &str) -> Result<Box<dyn Read + Send>, TransferError>;

    /// Returns `None` when the source ignores a range request and serves whole files only.
    fn try_get_range(
        &self,
        url: &str,
        offset: u64,
        length: u64,
    ) -> Result<Option<RangeResponse>, TransferError>;
}

struct RangeResponse {
    reader: Box<dyn Read + Send>,
    range: Range<u64>,
    total_size: u64,
}

/// How a file is split across concurrent ranged requests.
///
/// At most `workers` ranges are scheduled ahead of the reader, so together with the range being
/// read a plan holds up to `(workers + 1) * range_bytes` in memory.
#[derive(Debug, Clone)]
struct RangePlan {
    /// Ranges fetched at once.
    workers: usize,
    /// Bytes one range asks for.
    range_bytes: u64,
    /// Attempts a range gets before the read fails.
    attempts: u32,
    /// Wait before retrying a range, multiplied by the attempt already spent.
    backoff: Duration,
}

impl Default for RangePlan {
    fn default() -> Self {
        Self {
            workers: 6,
            range_bytes: 8 * 1024 * 1024,
            attempts: 3,
            backoff: Duration::from_millis(400),
        }
    }
}

/// Opens `url` for reading under the default [`RangePlan`].
pub fn open(
    client: &ReqwestTransferClient,
    url: &str,
    size_bytes: Option<u64>,
) -> Result<Box<dyn Read + Send>, TransferError> {
    open_with_plan(client, url, size_bytes, &RangePlan::default())
}

/// Opens `url` for reading, splitting it across concurrent ranged requests when its announced
/// size makes that worth doing and the source serves ranges.
///
/// Falls back to one whole-file request when the size is unknown, when the file fits in a single
/// range, or when the source answers a ranged request with the whole file.
fn open_with_plan<C: RangeSource>(
    client: &C,
    url: &str,
    size_bytes: Option<u64>,
    plan: &RangePlan,
) -> Result<Box<dyn Read + Send>, TransferError> {
    let split = size_bytes.filter(|size| *size > plan.range_bytes);
    let Some(size_bytes) = split else {
        return client.get_whole_reader(url);
    };

    // The first range is the file's own first chunk, and fetching it here answers whether the
    // source serves ranges before any worker is spawned.
    let stop = Arc::new(AtomicBool::new(false));
    let first_range = 0..plan.range_bytes;
    match ResumableRange::open(
        client,
        url,
        first_range,
        size_bytes,
        plan,
        Arc::clone(&stop),
    ) {
        Ok(Some(first)) => Ok(Box::new(RangedReader::new(
            client, url, size_bytes, plan, first, stop,
        ))),
        Ok(None) => client.get_whole_reader(url),
        Err(error) => Err(error),
    }
}

/// Reads a file's ranges as they arrive and hands the bytes on in order.
struct RangedReader<C> {
    ranges: usize,
    next: usize,
    current: Box<dyn Read + Send>,
    arrived: BTreeMap<usize, Vec<u8>>,
    results: Receiver<Result<(usize, Vec<u8>), TransferError>>,
    jobs: Sender<usize>,
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
    jobs: Receiver<usize>,
    results: Sender<Result<(usize, Vec<u8>), TransferError>>,
}

impl<C: RangeSource> RangedReader<C> {
    fn new(
        client: &C,
        url: &str,
        size_bytes: u64,
        plan: &RangePlan,
        first: ResumableRange<C>,
        stop: Arc<AtomicBool>,
    ) -> Self {
        let ranges = size_bytes.div_ceil(plan.range_bytes) as usize;
        let scheduled = plan.workers.min(ranges - 1);
        let (jobs, pending_jobs) = bounded(plan.workers);
        for index in 1..=scheduled {
            jobs.send(index).expect("range job receiver is alive");
        }
        let (completed_ranges, results) = bounded(plan.workers);

        Self {
            ranges,
            next: 1,
            current: Box::new(first),
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
                jobs: pending_jobs,
                results: completed_ranges,
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
            let jobs = start.jobs.clone();
            let stop = Arc::clone(&self.stop);
            let results = start.results.clone();
            let size_bytes = start.size_bytes;
            thread::spawn(move || {
                fetch_ranges(&client, &url, size_bytes, &plan, &jobs, &stop, &results)
            });
        }
    }

    fn schedule_next(&mut self) -> io::Result<()> {
        if self.next_to_schedule < self.ranges {
            self.jobs.send(self.next_to_schedule).map_err(|_| {
                io::Error::other("range workers stopped before all ranges were scheduled")
            })?;
            self.next_to_schedule += 1;
        }
        Ok(())
    }

    /// The bytes of one range, waiting for it when it has not arrived yet.
    fn take(&mut self, index: usize) -> io::Result<Vec<u8>> {
        if let Some(bytes) = self.arrived.remove(&index) {
            self.schedule_next()?;
            return Ok(bytes);
        }
        loop {
            match self.results.recv() {
                Ok(Ok((arrived, bytes))) if arrived == index => {
                    self.schedule_next()?;
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

/// Streams one range and resumes its unread suffix if the response body is interrupted.
struct ResumableRange<C> {
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

impl<C: RangeSource> ResumableRange<C> {
    fn open(
        client: &C,
        url: &str,
        range: Range<u64>,
        expected_size: u64,
        plan: &RangePlan,
        stop: Arc<AtomicBool>,
    ) -> Result<Option<Self>, TransferError> {
        let mut spent = 0;
        let Some(response) = request_with_retry(
            client,
            url,
            range.clone(),
            expected_size,
            plan,
            &stop,
            &mut spent,
        )?
        else {
            return Ok(None);
        };

        Ok(Some(Self {
            client: client.clone(),
            url: url.to_string(),
            expected_size,
            plan: plan.clone(),
            reader: response.reader,
            next_offset: response.range.start,
            end_offset: response.range.end,
            spent,
            stop,
        }))
    }

    fn resume(&mut self, error: TransferError) -> io::Result<()> {
        if self.spent >= self.plan.attempts || self.stop.load(Ordering::Relaxed) {
            return Err(io::Error::other(error));
        }
        let response = request_with_retry(
            &self.client,
            &self.url,
            self.next_offset..self.end_offset,
            self.expected_size,
            &self.plan,
            &self.stop,
            &mut self.spent,
        )
        .map_err(io::Error::other)?
        .ok_or_else(|| {
            io::Error::other(format!(
                "source stopped serving ranges after an interrupted response: {error}"
            ))
        })?;
        self.reader = response.reader;
        Ok(())
    }
}

impl<C: RangeSource> Read for ResumableRange<C> {
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
                Ok(0) => self.resume(TransferError::Transport(format!(
                    "range at {} ended before {}",
                    self.next_offset, self.end_offset
                )))?,
                Ok(read) => {
                    self.next_offset += read as u64;
                    return Ok(read);
                }
                Err(error) => self.resume(TransferError::Transport(error.to_string()))?,
            }
        }
    }
}

impl<C> Drop for RangedReader<C> {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Claims ranges until they run out, the reader goes away, or one of them cannot be read.
fn fetch_ranges<C: RangeSource>(
    client: &C,
    url: &str,
    size_bytes: u64,
    plan: &RangePlan,
    jobs: &Receiver<usize>,
    stop: &Arc<AtomicBool>,
    results: &Sender<Result<(usize, Vec<u8>), TransferError>>,
) {
    while !stop.load(Ordering::Relaxed) {
        let Ok(index) = jobs.recv() else {
            return;
        };
        if stop.load(Ordering::Relaxed) {
            return;
        }

        let offset = index as u64 * plan.range_bytes;
        debug_assert!(offset < size_bytes);

        let length = plan.range_bytes.min(size_bytes - offset);
        let result = read_range(client, url, offset, length, size_bytes, plan, stop);
        let failed = result.is_err();
        if results.send(result.map(|bytes| (index, bytes))).is_err() || failed {
            if failed {
                stop.store(true, Ordering::Relaxed);
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
    stop: &Arc<AtomicBool>,
) -> Result<Vec<u8>, TransferError> {
    let range = offset..offset + length;
    let Some(mut reader) =
        ResumableRange::open(client, url, range, expected_size, plan, Arc::clone(stop))?
    else {
        return Err(TransferError::Transport(
            "source stopped serving ranges".into(),
        ));
    };

    let mut bytes = Vec::with_capacity(length as usize);
    reader
        .read_to_end(&mut bytes)
        .map_err(|error| TransferError::Transport(error.to_string()))?;
    Ok(bytes)
}

fn request_with_retry<C: RangeSource>(
    client: &C,
    url: &str,
    range: Range<u64>,
    expected_size: u64,
    plan: &RangePlan,
    stop: &AtomicBool,
    spent: &mut u32,
) -> Result<Option<RangeResponse>, TransferError> {
    loop {
        *spent += 1;
        match request_once(client, url, range.clone(), expected_size) {
            Ok(response) => return Ok(response),
            Err(error) if *spent >= plan.attempts || stop.load(Ordering::Relaxed) => {
                return Err(error);
            }
            Err(_) => thread::sleep(plan.backoff * *spent),
        }
    }
}

fn request_once<C: RangeSource>(
    client: &C,
    url: &str,
    expected_range: Range<u64>,
    expected_size: u64,
) -> Result<Option<RangeResponse>, TransferError> {
    let length = expected_range.end - expected_range.start;
    let Some(response) = client.try_get_range(url, expected_range.start, length)? else {
        return Ok(None);
    };
    if response.range != expected_range {
        return Err(TransferError::Transport(format!(
            "requested range {expected_range:?}, source answered {:?}",
            response.range
        )));
    }
    if response.total_size != expected_size {
        return Err(TransferError::Transport(format!(
            "source size is {} bytes, expected {expected_size} bytes",
            response.total_size
        )));
    }

    Ok(Some(response))
}

impl RangeSource for ReqwestTransferClient {
    fn get_whole_reader(&self, url: &str) -> Result<Box<dyn Read + Send>, TransferError> {
        self.get_whole_reader(url)
    }

    fn try_get_range(
        &self,
        url: &str,
        offset: u64,
        length: u64,
    ) -> Result<Option<RangeResponse>, TransferError> {
        let last = offset + length.saturating_sub(1);
        let response = self
            .http
            .get(url)
            .header(reqwest::header::RANGE, format!("bytes={offset}-{last}"))
            .send()
            .map_err(|error| TransferError::Transport(error.to_string()))?;

        if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
            let (range, total_size) = parse_content_range(&response)?;
            return Ok(Some(RangeResponse {
                reader: Box::new(response),
                range,
                total_size,
            }));
        }
        // Any other success is the whole file: the source ignored the header.
        if response.status().is_success() {
            return Ok(None);
        }
        Err(TransferError::Transport(
            response.error_for_status().err().unwrap().to_string(),
        ))
    }
}

fn parse_content_range(
    response: &reqwest::blocking::Response,
) -> Result<(Range<u64>, u64), TransferError> {
    let value = response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| TransferError::Transport("partial response omitted Content-Range".into()))?;
    parse_content_range_value(value)
}

fn parse_content_range_value(value: &str) -> Result<(Range<u64>, u64), TransferError> {
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
    use super::*;
    use std::io::Cursor;
    use std::sync::atomic::AtomicUsize;
    use std::time::Instant;

    const TEST_RANGE_BYTES: u64 = 1024;

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
        first_chunk_calls: Arc<AtomicUsize>,
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
                first_chunk_calls: Arc::new(AtomicUsize::new(0)),
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

        fn try_get_range(
            &self,
            _url: &str,
            offset: u64,
            length: u64,
        ) -> Result<Option<RangeResponse>, TransferError> {
            let index = self.range_calls.fetch_add(1, Ordering::Relaxed);
            if offset < TEST_RANGE_BYTES {
                self.first_chunk_calls.fetch_add(1, Ordering::Relaxed);
            }
            if !self.stagger.is_zero() {
                thread::sleep(self.stagger * (8 - (index as u32).min(7)));
            }
            match self.ranges {
                Ranges::Ignored => Ok(None),
                Ranges::Failing(at) if at as u64 == offset => {
                    Err(TransferError::Transport("nope".into()))
                }
                served => {
                    let start = offset as usize;
                    let end = (start + length as usize).min(self.bytes.len());
                    let end = if served == Ranges::Short && offset < TEST_RANGE_BYTES {
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
                    Ok(Some(RangeResponse {
                        reader,
                        range: offset..offset + length,
                        total_size,
                    }))
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
            workers: 4,
            range_bytes: TEST_RANGE_BYTES,
            attempts: 2,
            backoff: Duration::from_millis(1),
        }
    }

    fn read_all(source: &Source, size: Option<u64>) -> io::Result<Vec<u8>> {
        let mut reader = open_with_plan(source, "url", size, &plan()).map_err(io::Error::other)?;
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

        let mut reader =
            open_with_plan(&source, "url", Some(size), &plan()).expect("the first range is served");
        let mut first_byte = [0];
        reader
            .read_exact(&mut first_byte)
            .expect("start the workers");
        let expected = 1 + plan().workers;
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

        let reader = open_with_plan(&source, "url", Some(size), &plan())
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
        assert_eq!(
            source.first_chunk_calls.load(Ordering::Relaxed),
            plan().attempts as usize
        );
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
