use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};

use futures::Stream;

use crate::bounds::{DynStream, MaybeSend};

/// Items the caller pulls.
///
/// Consume it as a [`Stream`], as a blocking iterator at a native sync edge, or by
/// [`try_next_now`](Streaming::try_next_now) from a loop that must not suspend. It is a plain
/// stream: a producer that should run on its own is spawned by whoever owns an executor and
/// feeds a [`channel`](Streaming::channel). Dropping the stream tells that producer to stop.
///
/// Like a [`Job`](crate::Job), it holds only what any executor can poll, so it can be consumed
/// from anywhere.
pub struct Streaming<T, E> {
    source: Source<T, E>,
}

enum Source<T, E> {
    Ready(vec::IntoIter<Result<T, E>>),
    Pending(DynStream<'static, Result<T, E>>),
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
    /// Wraps `stream` as items to be pulled.
    pub fn new<S>(stream: S) -> Self
    where
        S: Stream<Item = Result<T, E>> + MaybeSend + 'static,
    {
        Self {
            source: Source::Pending(Box::pin(stream)),
        }
    }

    /// Creates a stream that already holds `items`. Allocates no channel.
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
    /// For a producer that runs on its own, such as an actor. `capacity` bounds how far it may
    /// run ahead of the consumer. Dropping the sink ends the stream.
    pub fn channel(capacity: usize) -> (StreamingSink<T, E>, Self)
    where
        T: Send + 'static,
        E: Send + 'static,
    {
        let (tx, rx) = async_channel::bounded(capacity);
        (StreamingSink { tx }, Self::new(rx))
    }

    /// Takes the next item if pulling once yields one, without waiting.
    ///
    /// `None` means nothing is ready *or* the stream has ended; poll it as a [`Stream`] to tell
    /// the two apart.
    pub fn try_next_now(&mut self) -> Option<Result<T, E>> {
        match &mut self.source {
            Source::Ready(items) => items.next(),
            Source::Pending(stream) => {
                let mut cx = Context::from_waker(Waker::noop());
                match stream.as_mut().poll_next(&mut cx) {
                    Poll::Ready(item) => item,
                    Poll::Pending => None,
                }
            }
        }
    }

    /// Consumes the stream as an iterator that waits for each item on the current thread.
    ///
    /// Present exactly where [`Job::block`](crate::Job::block) is, for the same reason.
    #[cfg(all(feature = "std", not(target_arch = "wasm32")))]
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
            Source::Pending(stream) => stream.as_mut().poll_next(cx),
        }
    }
}

impl<T, E> fmt::Debug for Streaming<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let source = match &self.source {
            Source::Ready(_) => "ready",
            Source::Pending(_) => "pending",
        };
        f.debug_struct("Streaming")
            .field("source", &source)
            .finish()
    }
}

/// A [`Streaming`] consumed synchronously; see [`Streaming::blocking_iter`].
#[cfg(all(feature = "std", not(target_arch = "wasm32")))]
pub struct BlockingIter<T, E> {
    inner: Streaming<T, E>,
}

#[cfg(all(feature = "std", not(target_arch = "wasm32")))]
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

    /// Feeds every item of `stream` through, ending with its error if it yields one, and stops
    /// early once the consumer is gone.
    pub async fn forward<S>(self, mut stream: S)
    where
        S: Stream<Item = Result<T, E>> + Unpin,
    {
        use futures::StreamExt;

        while let Some(item) = stream.next().await {
            match item {
                Ok(item) => {
                    if self.send(item).await.is_err() {
                        return;
                    }
                }
                Err(error) => {
                    self.fail(error).await;
                    return;
                }
            }
        }
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
    use futures::executor::block_on;
    use futures::{StreamExt, stream};

    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct Failed;

    #[test]
    fn given_ready_stream_when_probed_then_items_come_out_in_order_then_none() {
        let mut stream = Streaming::<_, Failed>::ready(vec![1, 2]);

        assert_eq!(stream.try_next_now(), Some(Ok(1)));
        assert_eq!(stream.try_next_now(), Some(Ok(2)));
        assert_eq!(stream.try_next_now(), None);
    }

    #[test]
    fn given_failed_stream_when_iterated_then_the_error_is_its_only_item() {
        let items: Vec<_> = Streaming::<u8, Failed>::failed(Failed)
            .blocking_iter()
            .collect();

        assert_eq!(items, vec![Err(Failed)]);
    }

    #[test]
    fn given_wrapped_stream_when_probed_per_tick_then_ready_items_come_out_and_pending_is_none() {
        let (release, gate) = futures::channel::oneshot::channel::<()>();
        let mut stream = Streaming::<u8, Failed>::new(
            stream::once(async move {
                gate.await.ok();
                Ok(1)
            })
            .chain(stream::iter([Ok(2)])),
        );

        assert_eq!(stream.try_next_now(), None);
        release.send(()).unwrap();

        assert_eq!(stream.try_next_now(), Some(Ok(1)));
        assert_eq!(stream.try_next_now(), Some(Ok(2)));
        assert_eq!(stream.try_next_now(), None);
    }

    #[test]
    fn given_sink_when_fed_and_dropped_then_blocking_iter_drains_and_ends() {
        let (sink, stream) = Streaming::<u8, Failed>::channel(4);

        sink.try_send(1).unwrap();
        sink.try_send(2).unwrap();
        drop(sink);

        let items: Vec<_> = stream.blocking_iter().collect();
        assert_eq!(items, vec![Ok(1), Ok(2)]);
    }

    #[test]
    fn given_full_channel_when_sending_without_waiting_then_item_comes_back() {
        let (sink, _stream) = Streaming::<u8, Failed>::channel(1);

        sink.try_send(1).unwrap();

        assert_eq!(sink.try_send(2), Err(TrySendError::Full(2)));
    }

    #[test]
    fn given_dropped_consumer_when_sending_then_sink_reports_closed() {
        let (sink, stream) = Streaming::<u8, Failed>::channel(1);

        drop(stream);

        assert_eq!(sink.try_send(1), Err(TrySendError::Closed(1)));
        assert_eq!(block_on(sink.send(2)), Err(Closed));
    }

    #[test]
    fn given_producer_faster_than_capacity_on_its_own_thread_then_every_item_arrives_in_order() {
        let (sink, stream) = Streaming::<u8, Failed>::channel(1);
        std::thread::spawn(move || block_on(sink.forward(stream::iter((0..5).map(Ok)))));

        let items: Vec<_> = stream.blocking_iter().collect::<Result<_, _>>().unwrap();

        assert_eq!(items, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn given_producer_that_fails_then_items_precede_the_error() {
        let (sink, stream) = Streaming::<u8, Failed>::channel(4);
        std::thread::spawn(move || {
            block_on(sink.forward(stream::iter([Ok(1), Err(Failed), Ok(3)])))
        });

        let items: Vec<_> = stream.blocking_iter().collect();

        assert_eq!(items, vec![Ok(1), Err(Failed)]);
    }

    #[test]
    fn given_dropped_stream_then_a_producer_on_its_own_thread_sees_the_channel_close() {
        let (sink, stream) = Streaming::<u8, Failed>::channel(1);
        let producer =
            std::thread::spawn(move || block_on(async { while sink.send(0).await.is_ok() {} }));

        drop(stream);

        producer
            .join()
            .expect("the producer stopped once the consumer was gone");
    }
}
