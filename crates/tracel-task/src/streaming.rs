use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::vec;

use futures::Stream;
use futures::future::{AbortHandle, Abortable};

use crate::spawn::{MaybeSend, Spawn};
use crate::task::Aborted;

/// A handle to a producer that is already running.
///
/// Consume it as a [`Stream`], as a blocking iterator at a native sync edge, or by
/// [`try_next_now`](Streaming::try_next_now) from a loop that must not suspend. Dropping the
/// handle lets the producer run on; [`cancel`](Streaming::cancel) stops it.
///
/// `Streaming<T, E>` is `Send` whenever `T` and `E` are, whatever produces them.
#[must_use = "the producer is already running; call `detach` to release the handle deliberately"]
pub struct Streaming<T, E = Aborted> {
    source: Source<T, E>,
}

enum Source<T, E> {
    Ready(vec::IntoIter<Result<T, E>>),
    Pending {
        rx: Pin<Box<async_channel::Receiver<Result<T, E>>>>,
        abort: Option<AbortHandle>,
    },
}

/// The producing half of [`Streaming::channel`].
///
/// Every method takes `&self`, so a producer can be shared or called from synchronous code.
pub struct StreamingSink<T, E> {
    tx: async_channel::Sender<Result<T, E>>,
}

/// The consumer is gone; the producer should stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Closed;

/// Why [`StreamingSink::try_send`] could not accept an item. Carries the item back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrySendError<T> {
    /// The consumer is behind and the channel is at capacity.
    Full(T),
    /// The consumer is gone.
    Closed(T),
}

impl<T, E> Streaming<T, E> {
    /// Creates a stream that already holds `items`. Allocates no channel and spawns nothing.
    pub fn ready(items: Vec<T>) -> Self {
        Self::from_result(Ok(items))
    }

    /// Creates a stream that already failed with `error`.
    pub fn failed(error: E) -> Self {
        Self::from_result(Err(error))
    }

    /// Creates a stream that already holds every item of `result`, or its single error.
    pub fn from_result(result: Result<Vec<T>, E>) -> Self {
        let items = match result {
            Ok(items) => items.into_iter().map(Ok).collect::<Vec<_>>(),
            Err(error) => vec![Err(error)],
        };
        Self {
            source: Source::Ready(items.into_iter()),
        }
    }

    /// Creates a pending stream and the [`StreamingSink`] that feeds it.
    ///
    /// `capacity` bounds how far the producer may run ahead of the consumer. Dropping the sink
    /// ends the stream.
    pub fn channel(capacity: usize) -> (StreamingSink<T, E>, Self) {
        let (tx, rx) = async_channel::bounded(capacity);
        let stream = Self {
            source: Source::Pending {
                rx: Box::pin(rx),
                abort: None,
            },
        };
        (StreamingSink { tx }, stream)
    }

    /// Starts the producer returned by `run` on `spawn` and returns the consuming handle.
    pub fn spawn<F, Fut>(spawn: &dyn Spawn, capacity: usize, run: F) -> Self
    where
        F: FnOnce(StreamingSink<T, E>) -> Fut,
        Fut: Future<Output = ()> + MaybeSend + 'static,
        T: Send + 'static,
        E: Send + 'static,
    {
        let (sink, mut stream) = Self::channel(capacity);
        let (handle, registration) = AbortHandle::new_pair();
        let work = Abortable::new(run(sink), registration);

        spawn.spawn(Box::pin(async move {
            let _ = work.await;
        }));

        if let Source::Pending { abort, .. } = &mut stream.source {
            *abort = Some(handle);
        }
        stream
    }

    /// Takes the next item if one has arrived, without waiting.
    ///
    /// `None` means nothing is ready *or* the stream has ended; poll it as a [`Stream`] to tell
    /// the two apart.
    pub fn try_next_now(&mut self) -> Option<Result<T, E>> {
        match &mut self.source {
            Source::Ready(items) => items.next(),
            Source::Pending { rx, .. } => rx.try_recv().ok(),
        }
    }

    /// Stops the producer and closes the channel. Items already buffered can still be taken.
    pub fn cancel(&mut self) {
        if let Source::Pending { rx, abort } = &mut self.source {
            if let Some(abort) = abort.take() {
                abort.abort();
            }
            rx.close();
        }
    }

    /// Releases the handle and lets the producer finish unobserved.
    pub fn detach(self) {}

    /// Consumes the stream as an iterator that waits for each item on the current thread.
    ///
    /// Native only, for the same reason as [`Task::block`](crate::Task::block).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn blocking_iter(self) -> BlockingIter<T, E> {
        BlockingIter { inner: self }
    }
}

impl<T, E> Unpin for Streaming<T, E> {}

impl<T, E> Stream for Streaming<T, E> {
    type Item = Result<T, E>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match &mut self.get_mut().source {
            Source::Ready(items) => Poll::Ready(items.next()),
            Source::Pending { rx, .. } => rx.as_mut().poll_next(cx),
        }
    }
}

impl<T, E> fmt::Debug for Streaming<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let source = match &self.source {
            Source::Ready(_) => "ready",
            Source::Pending { .. } => "pending",
        };
        f.debug_struct("Streaming")
            .field("source", &source)
            .finish()
    }
}

/// A [`Streaming`] consumed synchronously; see [`Streaming::blocking_iter`].
#[cfg(not(target_arch = "wasm32"))]
pub struct BlockingIter<T, E> {
    inner: Streaming<T, E>,
}

#[cfg(not(target_arch = "wasm32"))]
impl<T, E> Iterator for BlockingIter<T, E> {
    type Item = Result<T, E>;

    fn next(&mut self) -> Option<Self::Item> {
        futures::executor::block_on(futures::StreamExt::next(&mut self.inner))
    }
}

impl<T, E> StreamingSink<T, E> {
    /// Emits `item`, waiting while the consumer is behind.
    pub async fn send(&self, item: T) -> Result<(), Closed> {
        self.tx.send(Ok(item)).await.map_err(|_| Closed)
    }

    /// Emits `item` without waiting, for producers that must not suspend.
    pub fn try_send(&self, item: T) -> Result<(), TrySendError<T>> {
        self.tx.try_send(Ok(item)).map_err(|e| match e {
            async_channel::TrySendError::Full(Ok(item)) => TrySendError::Full(item),
            async_channel::TrySendError::Closed(Ok(item)) => TrySendError::Closed(item),
            async_channel::TrySendError::Full(Err(_))
            | async_channel::TrySendError::Closed(Err(_)) => {
                unreachable!("try_send only offers `Ok` items")
            }
        })
    }

    /// Ends the stream with `error`.
    pub async fn fail(self, error: E) {
        let _ = self.tx.send(Err(error)).await;
    }
}

impl<T, E> fmt::Debug for StreamingSink<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StreamingSink")
            .field("capacity", &self.tx.capacity())
            .field("queued", &self.tx.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    use super::*;
    use crate::spawn::ThreadSpawn;

    fn set_within(flag: &AtomicBool, secs: u64) -> bool {
        let deadline = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < deadline {
            if flag.load(Ordering::SeqCst) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    #[test]
    fn given_ready_stream_when_probed_then_items_come_out_in_order_then_none() {
        let mut stream = Streaming::<_, Aborted>::ready(vec![1, 2]);

        assert_eq!(stream.try_next_now(), Some(Ok(1)));
        assert_eq!(stream.try_next_now(), Some(Ok(2)));
        assert_eq!(stream.try_next_now(), None);
    }

    #[test]
    fn given_failed_stream_when_iterated_then_the_error_is_its_only_item() {
        let items: Vec<_> = Streaming::<u8, Aborted>::failed(Aborted)
            .blocking_iter()
            .collect();

        assert_eq!(items, vec![Err(Aborted)]);
    }

    #[test]
    fn given_sink_when_fed_and_dropped_then_blocking_iter_drains_and_ends() {
        let (sink, stream) = Streaming::<u8, Aborted>::channel(4);

        sink.try_send(1).unwrap();
        sink.try_send(2).unwrap();
        drop(sink);

        let items: Vec<_> = stream.blocking_iter().collect();
        assert_eq!(items, vec![Ok(1), Ok(2)]);
    }

    #[test]
    fn given_full_channel_when_sending_without_waiting_then_item_comes_back() {
        let (sink, _stream) = Streaming::<u8, Aborted>::channel(1);

        sink.try_send(1).unwrap();

        assert_eq!(sink.try_send(2), Err(TrySendError::Full(2)));
    }

    #[test]
    fn given_dropped_consumer_when_sending_then_sink_reports_closed() {
        let (sink, stream) = Streaming::<u8, Aborted>::channel(1);

        drop(stream);

        assert_eq!(sink.try_send(1), Err(TrySendError::Closed(1)));
        assert_eq!(futures::executor::block_on(sink.send(2)), Err(Closed));
    }

    #[test]
    fn given_producer_faster_than_capacity_when_consumed_then_every_item_arrives_in_order() {
        let stream = Streaming::<u8, Aborted>::spawn(&ThreadSpawn, 1, |sink| async move {
            for item in 0..5 {
                if sink.send(item).await.is_err() {
                    return;
                }
            }
        });

        let items: Vec<_> = stream.blocking_iter().collect::<Result<_, _>>().unwrap();

        assert_eq!(items, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn given_producer_that_fails_when_consumed_then_items_precede_the_error() {
        let stream = Streaming::<u8, Aborted>::spawn(&ThreadSpawn, 4, |sink| async move {
            sink.send(1).await.ok();
            sink.fail(Aborted).await;
        });

        let items: Vec<_> = stream.blocking_iter().collect();

        assert_eq!(items, vec![Ok(1), Err(Aborted)]);
    }

    #[test]
    fn given_cancelled_spawned_stream_then_producer_is_dropped_at_its_next_suspension() {
        struct DropFlag(Arc<AtomicBool>);
        impl Drop for DropFlag {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let dropped = Arc::new(AtomicBool::new(false));
        let flag = DropFlag(dropped.clone());
        let mut stream = Streaming::<u8, Aborted>::spawn(&ThreadSpawn, 1, |sink| async move {
            let _flag = flag;
            loop {
                if sink.send(0).await.is_err() {
                    return;
                }
            }
        });

        stream.cancel();

        assert!(
            set_within(&dropped, 5),
            "producer kept running after cancel"
        );
    }

    #[test]
    fn given_cancelled_channel_stream_then_buffered_items_drain_and_new_sends_are_refused() {
        let (sink, mut stream) = Streaming::<u8, Aborted>::channel(2);
        sink.try_send(1).unwrap();

        stream.cancel();

        assert_eq!(sink.try_send(2), Err(TrySendError::Closed(2)));
        assert_eq!(stream.try_next_now(), Some(Ok(1)));
        assert_eq!(stream.try_next_now(), None);
    }
}
