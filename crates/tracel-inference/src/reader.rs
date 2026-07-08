use std::sync::Mutex;

/// Errors that can occur when reading from an inference input stream.
///
/// A normal end of stream is signalled by `Ok(None)` from
/// [`InferenceReaderChannel::read`], not by an error.
#[derive(Debug, thiserror::Error)]
pub enum InferenceReaderError {
    /// The input stream was closed before it finished producing items.
    #[error("inference input stream closed unexpectedly")]
    Closed,
    /// A raw input message could not be decoded into the expected input type.
    #[error("failed to decode inference input: {0}")]
    Decode(Box<dyn std::error::Error + Send + Sync>),
    /// Any other transport-level failure.
    #[error("unknown error: {0}")]
    Unknown(Box<dyn std::error::Error + Send + Sync>),
}

/// Source of typed input items for an inference task.
///
/// Each transport (manual iterator, HTTP request body, websocket, ...) implements this to frame and
/// decode its byte stream into discrete typed items.
pub trait InferenceReaderChannel<I>: Send {
    /// Return the next input item, or `None` when the input stream is complete.
    fn read(&self) -> Result<Option<I>, InferenceReaderError>;
}

/// The input handed to [`Inference::infer`](crate::Inference::infer).
///
/// A pull-based reader: call [`recv`](Self::recv) to pull the next item, or iterate to consume the
/// stream. For the single-request case use [`once`](Self::once).
pub struct InferenceInput<I> {
    channel: Box<dyn InferenceReaderChannel<I>>,
}

impl<I> InferenceInput<I> {
    pub(crate) fn new(channel: Box<dyn InferenceReaderChannel<I>>) -> Self {
        Self { channel }
    }

    pub(crate) fn from_channel<C>(channel: C) -> Self
    where
        C: InferenceReaderChannel<I> + 'static,
    {
        Self::new(Box::new(channel))
    }

    /// Pull the next input item, or `None` once the stream is complete.
    pub fn recv(&self) -> Result<Option<I>, InferenceReaderError> {
        self.channel.read()
    }

    /// Build an input stream that yields a single item and then ends.
    pub fn once(input: I) -> Self
    where
        I: Send + 'static,
    {
        Self::from_channel(OnceReaderChannel::new(input))
    }
}

/// Iterating an [`InferenceInput`] pulls items until the stream ends. Read errors terminate the
/// iteration; use [`recv`](InferenceInput::recv) directly if you need to observe them.
impl<I> Iterator for InferenceInput<I> {
    type Item = I;

    fn next(&mut self) -> Option<I> {
        self.channel.read().ok().flatten()
    }
}

/// A channel that yields a single item and then reports end of stream.
struct OnceReaderChannel<I> {
    item: Mutex<Option<I>>,
}

impl<I> OnceReaderChannel<I> {
    fn new(item: I) -> Self {
        Self {
            item: Mutex::new(Some(item)),
        }
    }
}

impl<I> InferenceReaderChannel<I> for OnceReaderChannel<I>
where
    I: Send,
{
    fn read(&self) -> Result<Option<I>, InferenceReaderError> {
        Ok(self.item.lock().unwrap().take())
    }
}

/// A channel that drains an iterator, one item per [`read`](InferenceReaderChannel::read).
///
/// Used by [`DirectInference::stream`](crate::stream::DirectInference::stream) to feed a manually
/// provided sequence of inputs into an inference task.
pub(crate) struct IterReaderChannel<It> {
    iter: Mutex<It>,
}

impl<It> IterReaderChannel<It> {
    pub(crate) fn new(iter: It) -> Self {
        Self {
            iter: Mutex::new(iter),
        }
    }
}

impl<It, I> InferenceReaderChannel<I> for IterReaderChannel<It>
where
    It: Iterator<Item = I> + Send,
    I: Send,
{
    fn read(&self) -> Result<Option<I>, InferenceReaderError> {
        Ok(self.iter.lock().unwrap().next())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn once_yields_single_item_then_ends() {
        let input = InferenceInput::once(42);
        assert_eq!(input.recv().unwrap(), Some(42));
        assert_eq!(input.recv().unwrap(), None);
    }

    #[test]
    fn iter_channel_drains_in_order() {
        let input = InferenceInput::from_channel(IterReaderChannel::new(vec![1, 2, 3].into_iter()));
        let collected: Vec<i32> = input.collect();
        assert_eq!(collected, vec![1, 2, 3]);
    }
}
