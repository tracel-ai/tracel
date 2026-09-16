use std::future::Future;
use std::io::{self, Read};
use std::time::Duration;

use bytes::Bytes;
use futures::{Stream, stream};
use tracel_task::{MaybeSend, MaybeSync};

mod http;
mod ranged;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod test_server;

pub use http::HttpTransferClient;

const TRANSFER_SECONDS_ALLOWED_PER_MEGABYTE: u64 = 10;

const MINIMUM_TRANSFER_TIMEOUT: Duration = Duration::from_secs(60);

#[cfg(not(target_arch = "wasm32"))]
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

const READ_CHUNK_BYTES: usize = 64 * 1024;

fn timeout_worth_allowing_a_transfer_of(size_bytes: Option<u64>) -> Duration {
    const BYTES_PER_MEGABYTE: u64 = 1024 * 1024;

    let Some(size_bytes) = size_bytes else {
        return Duration::from_secs(60 * 60);
    };

    let megabytes = size_bytes.div_ceil(BYTES_PER_MEGABYTE);
    let allowed =
        Duration::from_secs(megabytes.saturating_mul(TRANSFER_SECONDS_ALLOWED_PER_MEGABYTE));

    allowed.max(MINIMUM_TRANSFER_TIMEOUT)
}

fn transport_failure(error: &dyn std::error::Error) -> TransferError {
    let mut described = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        described.push_str(": ");
        described.push_str(&cause.to_string());
        source = cause.source();
    }

    TransferError::Transport(described)
}

/// Watches a transfer as it runs, and can stop it.
///
/// Every method defaults to doing nothing, so an implementation only has to define the events it
/// cares about. Callbacks run on the transferring thread and block it, so an implementation that
/// does real work should hand it off.
pub trait TransferObserver: Send {
    /// Returns whether the active transfer should stop.
    ///
    /// Polled at file, part, and reader boundaries. Implementations should make this query cheap
    /// and may update its result from another thread or from a progress callback.
    fn is_cancelled(&self) -> bool {
        false
    }

    /// A file is about to be transferred. `total_bytes` is absent when nothing announced the
    /// size, as for an artifact published without a manifest.
    fn file_started(&mut self, rel_path: &str, total_bytes: Option<u64>) {
        let _ = (rel_path, total_bytes);
    }

    /// Bytes have moved for the file being transferred. `transferred_bytes` is the running total
    /// for that file, not an increment.
    fn file_progress(&mut self, rel_path: &str, transferred_bytes: u64) {
        let _ = (rel_path, transferred_bytes);
    }

    /// A file has been transferred in full.
    fn file_completed(&mut self, rel_path: &str, transferred_bytes: u64) {
        let _ = (rel_path, transferred_bytes);
    }
}

/// Transfers no one is watching.
impl TransferObserver for () {}

/// So a borrowed observer can be handed to code that wants to own one.
impl<O: TransferObserver + ?Sized> TransferObserver for &mut O {
    fn is_cancelled(&self) -> bool {
        (**self).is_cancelled()
    }

    fn file_started(&mut self, rel_path: &str, total_bytes: Option<u64>) {
        (**self).file_started(rel_path, total_bytes);
    }

    fn file_progress(&mut self, rel_path: &str, transferred_bytes: u64) {
        (**self).file_progress(rel_path, transferred_bytes);
    }

    fn file_completed(&mut self, rel_path: &str, transferred_bytes: u64) {
        (**self).file_completed(rel_path, transferred_bytes);
    }
}

/// So an observer can be handed to a transfer and still be read by whoever started it.
impl<O: TransferObserver + ?Sized> TransferObserver for std::sync::Arc<std::sync::Mutex<O>> {
    fn is_cancelled(&self) -> bool {
        self.lock()
            .map(|observer| observer.is_cancelled())
            .unwrap_or(true)
    }

    fn file_started(&mut self, rel_path: &str, total_bytes: Option<u64>) {
        if let Ok(mut observer) = self.lock() {
            observer.file_started(rel_path, total_bytes);
        }
    }

    fn file_progress(&mut self, rel_path: &str, transferred_bytes: u64) {
        if let Ok(mut observer) = self.lock() {
            observer.file_progress(rel_path, transferred_bytes);
        }
    }

    fn file_completed(&mut self, rel_path: &str, transferred_bytes: u64) {
        if let Ok(mut observer) = self.lock() {
            observer.file_completed(rel_path, transferred_bytes);
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TransferError {
    #[error("Transport error: {0}")]
    Transport(String),
}

/// A downloaded body.
pub type ByteStream = tracel_task::DynStream<'static, Result<Bytes, TransferError>>;

/// Moves bytes to and from URLs.
///
/// Implementations own the transport; callers own what the bytes mean.
pub trait TransferClient: Clone + MaybeSend + MaybeSync + 'static {
    /// The body of a download.
    type Body: Stream<Item = Result<Bytes, TransferError>> + MaybeSend + Unpin + 'static;

    /// Downloads `url`.
    ///
    /// `expected_size_bytes` is the size declared by the manifest, `None` when none was. It
    /// selects the transfer strategy and the deadline.
    fn get(
        &self,
        url: &str,
        expected_size_bytes: Option<u64>,
    ) -> impl Future<Output = Result<Self::Body, TransferError>> + MaybeSend;

    /// Uploads `size_bytes` of `body` to `url`.
    fn put<B>(
        &self,
        url: &str,
        body: B,
        size_bytes: u64,
    ) -> impl Future<Output = Result<(), TransferError>> + MaybeSend
    where
        B: Stream<Item = Result<Bytes, io::Error>> + MaybeSend + 'static;
}

/// Reads `reader` as a stream of chunks. The stream ends at the first error.
pub fn reader_stream<R>(reader: R) -> impl Stream<Item = Result<Bytes, io::Error>> + MaybeSend
where
    R: Read + MaybeSend + 'static,
{
    stream::unfold(Some(reader), |reader| async move {
        let mut reader = reader?;
        let mut buf = vec![0u8; READ_CHUNK_BYTES];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => return None,
                Ok(read) => {
                    buf.truncate(read);
                    return Some((Ok(Bytes::from(buf)), Some(reader)));
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Some((Err(error), None)),
            }
        }
    })
}
