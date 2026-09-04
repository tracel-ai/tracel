//! Reading one file through several concurrent ranged requests.
//!
//! The bytes come back in order through a plain [`Read`], so everything downstream — bundle
//! sinks, checksum verification, progress and cancellation — is unchanged by the split.

use std::collections::{BTreeMap, VecDeque};
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use crate::TransferError;

pub(crate) trait RangeSource: Clone + Send + Sync + 'static {
    fn get_whole_reader(&self, url: &str) -> Result<Box<dyn Read + Send>, TransferError>;

    fn get_range(
        &self,
        url: &str,
        offset: u64,
        length: u64,
    ) -> Result<Box<dyn Read + Send>, RangeSourceError>;
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
    pub workers: usize,
    /// Bytes one range asks for.
    pub range_bytes: u64,
    /// Attempts a range gets before the read fails.
    pub attempts: u32,
    /// Wait before retrying a range, multiplied by the attempt already spent.
    pub backoff: Duration,
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
    let split = size_bytes.filter(|size| *size > plan.range_bytes && plan.workers > 0);
    let Some(size_bytes) = split else {
        return client.get_whole_reader(url);
    };

    // The first range is the file's own first chunk, and fetching it here answers whether the
    // source serves ranges before any worker is spawned.
    let stop = Arc::new(AtomicBool::new(false));
    match read_range(client, url, 0, plan.range_bytes, plan, &stop) {
        Ok(first) => Ok(Box::new(RangedReader::spawn(
            client, url, size_bytes, plan, first, stop,
        ))),
        Err(RangeSourceError::Unsupported) => client.get_whole_reader(url),
        Err(RangeSourceError::Transfer(error)) => Err(error),
    }
}

/// Reads a file's ranges as they arrive and hands the bytes on in order.
struct RangedReader {
    ranges: usize,
    next: usize,
    current: io::Cursor<Vec<u8>>,
    arrived: BTreeMap<usize, Vec<u8>>,
    results: Receiver<Result<(usize, Vec<u8>), RangeSourceError>>,
    jobs: Arc<RangeJobs>,
    next_to_schedule: usize,
    stop: Arc<AtomicBool>,
}

impl RangedReader {
    fn spawn<C: RangeSource>(
        client: &C,
        url: &str,
        size_bytes: u64,
        plan: &RangePlan,
        first: Vec<u8>,
        stop: Arc<AtomicBool>,
    ) -> Self {
        let ranges = size_bytes.div_ceil(plan.range_bytes) as usize;
        let scheduled = plan.workers.min(ranges - 1);
        let jobs = Arc::new(RangeJobs::new(1..=scheduled));
        let (sender, results) = sync_channel(plan.workers);

        for _ in 0..scheduled {
            let client = client.clone();
            let url = url.to_string();
            let plan = plan.clone();
            let jobs = Arc::clone(&jobs);
            let stop = Arc::clone(&stop);
            let sender = sender.clone();
            thread::spawn(move || {
                fetch_ranges(&client, &url, size_bytes, &plan, &jobs, &stop, &sender)
            });
        }
        // The reader's own handle would otherwise keep it from ever disconnecting.
        drop(sender);

        Self {
            ranges,
            next: 1,
            current: io::Cursor::new(first),
            arrived: BTreeMap::new(),
            results,
            jobs,
            next_to_schedule: scheduled + 1,
            stop,
        }
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

impl Read for RangedReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
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
            self.current = io::Cursor::new(Vec::new());
            let bytes = self.take(self.next)?;
            self.next += 1;
            self.current = io::Cursor::new(bytes);
        }
    }
}

impl Drop for RangedReader {
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
        let offset = index as u64 * plan.range_bytes;
        debug_assert!(offset < size_bytes);

        let length = plan.range_bytes.min(size_bytes - offset);
        let result = read_range(client, url, offset, length, plan, stop);
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
    plan: &RangePlan,
    stop: &AtomicBool,
) -> Result<Vec<u8>, RangeSourceError> {
    let mut spent = 0;
    loop {
        let attempt = read_range_once(client, url, offset, length);
        spent += 1;
        match attempt {
            Ok(bytes) => return Ok(bytes),
            Err(RangeSourceError::Unsupported) => return Err(RangeSourceError::Unsupported),
            Err(error) if spent >= plan.attempts || stop.load(Ordering::Relaxed) => {
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
) -> Result<Vec<u8>, RangeSourceError> {
    let mut bytes = Vec::with_capacity(length as usize);
    client
        .get_range(url, offset, length)?
        .take(length)
        .read_to_end(&mut bytes)
        .map_err(|error| TransferError::Transport(error.to_string()))?;

    // A short range would silently truncate the file, and the checksum would be blamed for it.
    if bytes.len() as u64 != length {
        return Err(TransferError::Transport(format!(
            "range at {offset} answered {} of {length} bytes",
            bytes.len()
        ))
        .into());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::atomic::AtomicUsize;

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
    }

    #[derive(Clone)]
    struct Source {
        bytes: Arc<Vec<u8>>,
        ranges: Ranges,
        range_calls: Arc<AtomicUsize>,
        whole_calls: Arc<AtomicUsize>,
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
        ) -> Result<Box<dyn Read + Send>, RangeSourceError> {
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
                    Ok(Box::new(Cursor::new(self.bytes[start..end].to_vec())))
                }
            }
        }
    }

    fn plan() -> RangePlan {
        RangePlan {
            workers: 4,
            range_bytes: 1024,
            attempts: 2,
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

        let reader = open_for_download_with_plan(&source, "url", Some(size), &plan())
            .expect("the first range is served");
        // The source is in-memory, so this is ample time for every scheduled worker to finish.
        thread::sleep(Duration::from_millis(20));

        assert_eq!(
            source.range_calls.load(Ordering::Relaxed),
            1 + plan().workers
        );
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

        assert!(error.to_string().contains("of 1024 bytes"), "{error}");
    }
}
