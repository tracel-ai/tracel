//! Concurrent ranged downloads presented as one in-order byte stream.

use std::future::Future;
use std::ops::Range;
use std::time::Duration;

use bytes::Bytes;
use futures::{Stream, StreamExt, TryStreamExt, stream};
use tracel_task::{MaybeSend, MaybeSync};

use super::{ByteStream, TransferError};

/// Backend primitives for a ranged download.
pub trait RangeSource: Clone + MaybeSend + MaybeSync + 'static {
    /// The body of one response.
    type Body: Stream<Item = Result<Bytes, TransferError>> + MaybeSend + Unpin + 'static;

    /// Requests the whole file.
    fn whole(
        &self,
        url: &str,
        expected_size_bytes: Option<u64>,
    ) -> impl Future<Output = Result<Self::Body, TransferError>> + MaybeSend;

    /// Requests `length` bytes from `offset`. Resolves to `None` if the source does not serve
    /// ranged requests.
    fn range(
        &self,
        url: &str,
        offset: u64,
        length: u64,
    ) -> impl Future<Output = Result<Option<RangeResponse<Self::Body>>, TransferError>> + MaybeSend;
}

/// A source's answer to one ranged request.
pub struct RangeResponse<B> {
    pub body: B,
    pub range: Range<u64>,
    pub total_size: u64,
}

/// Configuration for a ranged download.
///
/// Buffered data is limited to about `(workers + 2) * range_bytes`: the range being read, the
/// one handed to the reader, and at most `workers` fetched ahead of them.
#[derive(Debug, Clone)]
pub struct RangePlan {
    /// Maximum number of ranges fetched ahead of the one being read.
    pub workers: usize,
    /// Requested bytes per range.
    pub range_bytes: u64,
    /// Maximum number of requests per range.
    pub attempts: u32,
    /// Base delay between retries.
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

/// Opens `url` using ranged requests when its expected size exceeds one range.
///
/// Falls back to a whole-file request when the size is unknown, the file fits in one range, or the
/// source does not serve ranged requests. The first range is requested before this returns, so
/// that fallback is decided here; nothing else is fetched until the stream is read.
///
/// Up to one more range than `workers` is in flight at once, advancing whenever the stream is
/// polled. Dropping the stream abandons the ranges in flight.
pub async fn open<C: RangeSource>(
    client: &C,
    url: &str,
    size_bytes: Option<u64>,
    plan: &RangePlan,
) -> Result<ByteStream, TransferError> {
    let Some(size_bytes) = size_bytes.filter(|size| *size > plan.range_bytes) else {
        return Ok(Box::pin(client.whole(url, size_bytes).await?));
    };

    let first = 0..plan.range_bytes;
    let mut spent = 0;
    let Some(opened) =
        request_with_retry(client, url, first.clone(), size_bytes, plan, &mut spent).await?
    else {
        return Ok(Box::pin(client.whole(url, Some(size_bytes)).await?));
    };

    let ranges = size_bytes.div_ceil(plan.range_bytes);
    let window = plan.workers + 1;
    let client = client.clone();
    let url = url.to_string();
    let plan = plan.clone();
    let first = read_range(
        client.clone(),
        url.clone(),
        first,
        size_bytes,
        plan.clone(),
        Some(opened),
        spent,
    );
    let rest = stream::iter(1..ranges).map(move |index| {
        let offset = index * plan.range_bytes;
        let length = plan.range_bytes.min(size_bytes - offset);
        read_range(
            client.clone(),
            url.clone(),
            offset..offset + length,
            size_bytes,
            plan.clone(),
            None,
            0,
        )
    });

    Ok(Box::pin(
        stream::iter(std::iter::once(first))
            .chain(rest)
            .buffered(window)
            .map_ok(Bytes::from),
    ))
}

/// Downloads one range in full, resuming its unread suffix if a response is interrupted.
async fn read_range<C: RangeSource>(
    client: C,
    url: String,
    range: Range<u64>,
    expected_size: u64,
    plan: RangePlan,
    opened: Option<RangeResponse<C::Body>>,
    mut spent: u32,
) -> Result<Vec<u8>, TransferError> {
    let mut body = match opened {
        Some(response) => response.body,
        None => {
            match request_with_retry(
                &client,
                &url,
                range.clone(),
                expected_size,
                &plan,
                &mut spent,
            )
            .await?
            {
                Some(response) => response.body,
                None => {
                    return Err(TransferError::Transport(
                        "source stopped serving ranges".into(),
                    ));
                }
            }
        }
    };

    let mut bytes = Vec::with_capacity((range.end - range.start) as usize);
    let mut next = range.start;
    loop {
        let interrupted = match body.next().await {
            Some(Ok(chunk)) => {
                if chunk.len() as u64 > range.end - next {
                    return Err(TransferError::Transport(format!(
                        "range ending at {} answered more bytes than requested",
                        range.end
                    )));
                }
                bytes.extend_from_slice(&chunk);
                next += chunk.len() as u64;
                if next < range.end {
                    continue;
                }
                return match body.next().await {
                    None => Ok(bytes),
                    Some(Ok(extra)) if extra.is_empty() => Ok(bytes),
                    Some(Ok(_)) => Err(TransferError::Transport(format!(
                        "range ending at {} answered more bytes than requested",
                        range.end
                    ))),
                    Some(Err(error)) => Err(error),
                };
            }
            Some(Err(error)) => error,
            None => TransferError::Transport(format!("range at {next} ended before {}", range.end)),
        };

        if spent >= plan.attempts {
            return Err(interrupted);
        }
        body = match request_with_retry(
            &client,
            &url,
            next..range.end,
            expected_size,
            &plan,
            &mut spent,
        )
        .await?
        {
            Some(response) => response.body,
            None => {
                return Err(TransferError::Transport(format!(
                    "source stopped serving ranges after an interrupted response: {interrupted}"
                )));
            }
        };
    }
}

async fn request_with_retry<C: RangeSource>(
    client: &C,
    url: &str,
    range: Range<u64>,
    expected_size: u64,
    plan: &RangePlan,
    spent: &mut u32,
) -> Result<Option<RangeResponse<C::Body>>, TransferError> {
    loop {
        *spent += 1;
        match request_once(client, url, range.clone(), expected_size).await {
            Ok(response) => return Ok(response),
            Err(error) if *spent >= plan.attempts => return Err(error),
            Err(_) => futures_timer::Delay::new(plan.backoff * *spent).await,
        }
    }
}

async fn request_once<C: RangeSource>(
    client: &C,
    url: &str,
    expected_range: Range<u64>,
    expected_size: u64,
) -> Result<Option<RangeResponse<C::Body>>, TransferError> {
    let length = expected_range.end - expected_range.start;
    let Some(response) = client.range(url, expected_range.start, length).await? else {
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

#[cfg(test)]
mod tests {
    use std::pin::Pin;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::task::{Context, Poll, Waker};
    use std::vec;

    use futures::executor::block_on;
    use futures::future;

    use super::*;

    const TEST_RANGE_BYTES: u64 = 1024;

    /// Test behavior for ranged requests.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Ranges {
        Served,
        /// Returns the entire file.
        Ignored,
        /// Returns one byte fewer than requested.
        Short,
        /// Fails at the given offset.
        Failing(usize),
        /// Reports a different total size.
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
        unfinished_drops: Arc<AtomicUsize>,
        /// How many polls the n-th ranged request stays pending.
        stagger: fn(usize) -> usize,
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
                unfinished_drops: Arc::new(AtomicUsize::new(0)),
                stagger: |_| 0,
            }
        }

        fn stagger(mut self, stagger: fn(usize) -> usize) -> Self {
            self.stagger = stagger;
            self
        }

        /// The first range's body is what `open` hands back already requested, so it carries
        /// the stagger that a ranged request would otherwise get.
        fn body(&self, range: Range<usize>, first: bool) -> Body {
            let chunks = self.bytes[range]
                .chunks(300)
                .map(|chunk| Ok(Bytes::copy_from_slice(chunk)))
                .collect::<Vec<_>>();
            Body {
                inner: stream::iter(chunks),
                reads: first.then(|| Arc::clone(&self.first_body_reads)),
                pending: if first { (self.stagger)(0) } else { 0 },
            }
        }
    }

    struct Body {
        inner: stream::Iter<vec::IntoIter<Result<Bytes, TransferError>>>,
        reads: Option<Arc<AtomicUsize>>,
        pending: usize,
    }

    impl Stream for Body {
        type Item = Result<Bytes, TransferError>;

        fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            let this = self.get_mut();
            if let Some(reads) = &this.reads {
                reads.fetch_add(1, Ordering::Relaxed);
            }
            if this.pending > 0 {
                this.pending -= 1;
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
            Pin::new(&mut this.inner).poll_next(cx)
        }
    }

    /// Resolves after `pending` polls — never, for `usize::MAX` — and counts being dropped
    /// before that.
    struct Stagger<T> {
        pending: usize,
        value: Option<T>,
        unfinished_drops: Arc<AtomicUsize>,
    }

    impl<T: Unpin> Future for Stagger<T> {
        type Output = T;

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
            let this = self.get_mut();
            if this.pending == usize::MAX {
                return Poll::Pending;
            }
            if this.pending > 0 {
                this.pending -= 1;
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
            Poll::Ready(this.value.take().expect("polled after completion"))
        }
    }

    impl<T> Drop for Stagger<T> {
        fn drop(&mut self) {
            if self.value.is_some() {
                self.unfinished_drops.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    impl RangeSource for Source {
        type Body = Body;

        fn whole(
            &self,
            _url: &str,
            _expected_size_bytes: Option<u64>,
        ) -> impl Future<Output = Result<Body, TransferError>> + MaybeSend {
            self.whole_calls.fetch_add(1, Ordering::Relaxed);
            future::ready(Ok(self.body(0..self.bytes.len(), false)))
        }

        fn range(
            &self,
            _url: &str,
            offset: u64,
            length: u64,
        ) -> impl Future<Output = Result<Option<RangeResponse<Body>>, TransferError>> + MaybeSend
        {
            let index = self.range_calls.fetch_add(1, Ordering::Relaxed);
            if offset < TEST_RANGE_BYTES {
                self.first_chunk_calls.fetch_add(1, Ordering::Relaxed);
            }
            let value = match self.ranges {
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
                    Ok(Some(RangeResponse {
                        body: self.body(start..end, offset == 0),
                        range: offset..offset + length,
                        total_size,
                    }))
                }
            };
            Stagger {
                pending: (self.stagger)(index),
                value: Some(value),
                unfinished_drops: Arc::clone(&self.unfinished_drops),
            }
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

    fn read_all(source: &Source, size: Option<u64>) -> Result<Vec<u8>, TransferError> {
        block_on(async {
            open(source, "url", size, &plan())
                .await?
                .map_ok(|chunk| chunk.to_vec())
                .try_concat()
                .await
        })
    }

    /// Earlier requests complete later than later ones.
    fn earlier_finish_later(index: usize) -> usize {
        8 - index.min(7)
    }

    #[test]
    fn ranges_arriving_out_of_order_are_read_in_order() {
        let source = Source::new(8 * 1024 + 500, Ranges::Served).stagger(earlier_finish_later);
        let size = source.bytes.len() as u64;

        let read = read_all(&source, Some(size)).expect("every range is served");

        assert_eq!(read, *source.bytes);
        assert_eq!(source.whole_calls.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn fetching_stays_within_the_read_window() {
        // Ten ranges; every one past the first stays pending.
        let source = Source::new(10 * TEST_RANGE_BYTES as usize, Ranges::Served)
            .stagger(|index| if index == 0 { 0 } else { usize::MAX });
        let size = source.bytes.len() as u64;
        let window = 1 + plan().workers;

        let mut body =
            block_on(open(&source, "url", Some(size), &plan())).expect("the first range is served");
        block_on(body.next()).expect("the first range").unwrap();
        let mut cx = Context::from_waker(Waker::noop());
        assert!(Pin::new(&mut body).poll_next(&mut cx).is_pending());

        // Polling past the first range started a window's worth and no more, however many
        // times the reader asks before one of them lands.
        assert_eq!(source.range_calls.load(Ordering::Relaxed), 1 + window);
        assert!(Pin::new(&mut body).poll_next(&mut cx).is_pending());
        assert_eq!(source.range_calls.load(Ordering::Relaxed), 1 + window);
        drop(body);
    }

    #[test]
    fn opening_does_not_read_a_body_or_fetch_ahead() {
        let source = Source::new(8 * 1024, Ranges::Served);
        let size = source.bytes.len() as u64;

        let body = block_on(open(&source, "url", Some(size), &plan()))
            .expect("the first range headers are served");

        assert_eq!(source.range_calls.load(Ordering::Relaxed), 1);
        assert_eq!(source.first_body_reads.load(Ordering::Relaxed), 0);
        drop(body);
    }

    #[test]
    fn dropping_the_stream_abandons_the_ranges_in_flight() {
        let source = Source::new(20 * 1024, Ranges::Served)
            .stagger(|index| if index == 0 { 0 } else { usize::MAX });
        let size = source.bytes.len() as u64;

        let mut body = block_on(open(&source, "url", Some(size), &plan())).unwrap();
        block_on(body.next()).unwrap().unwrap();
        let mut cx = Context::from_waker(Waker::noop());
        assert!(Pin::new(&mut body).poll_next(&mut cx).is_pending());
        let in_flight = source.range_calls.load(Ordering::Relaxed) - 1;
        assert!(in_flight > 0);
        drop(body);

        assert_eq!(
            source.unfinished_drops.load(Ordering::Relaxed),
            in_flight,
            "the ranges in flight kept running after the stream was dropped"
        );
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
}
