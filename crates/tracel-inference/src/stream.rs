use std::time::Duration;

use tracel_task::Streaming;

use crate::{OutputWriter, output::OutputWriterError};

type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

/// Outputs handed over a channel: the writer half is an [`OutputWriter`] for an inference to
/// fill, the reader half is a [`Streaming`] the caller pulls from wherever it runs.
///
/// The channel is unbounded, so an inference never waits on its consumer. Dropping the stream
/// cancels the request: the writer reports [`OutputWriterError::Cancelled`] on its next write.
pub fn channel<O>() -> (OutputChannel<O>, Streaming<O, BoxError>)
where
    O: Send + 'static,
{
    let (tx, rx) = async_channel::unbounded();
    (OutputChannel { tx }, Streaming::new(rx))
}

/// The writer half of [`channel`].
pub struct OutputChannel<O> {
    tx: async_channel::Sender<Result<O, BoxError>>,
}

impl<O> OutputWriter<O> for OutputChannel<O>
where
    O: Send + Sync + 'static,
{
    fn write(&self, output: O) -> Result<(), OutputWriterError> {
        self.tx
            .try_send(Ok(output))
            .map_err(|_| OutputWriterError::Cancelled)
    }

    fn error(&self, error: BoxError) -> Result<(), OutputWriterError> {
        self.tx
            .try_send(Err(error))
            .map_err(|_| OutputWriterError::Cancelled)
    }

    fn finish(&self, _duration: Duration) {
        self.tx.close();
    }
}
