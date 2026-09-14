use std::future::Future;
use std::io::{self, Read};
use std::sync::Arc;
use std::thread;

use bytes::Bytes;
use futures::StreamExt;
use futures::channel::oneshot;
use tokio::runtime::{Builder, Handle};
use tracel_task::{BlockingIter, Spawn, SpawnedFuture, Streaming, StreamingSink};

use super::{ByteStream, HttpTransferClient, TransferClient, TransferError, reader_stream};

/// A transfer client for callers that block.
///
/// Drives an [`HttpTransferClient`] on a private runtime thread, so a caller needs neither a
/// runtime nor an executor of its own. Clones share that thread.
#[derive(Clone)]
pub struct ReqwestTransferClient {
    http: HttpTransferClient,
    driver: Arc<Driver>,
}

impl ReqwestTransferClient {
    pub fn new() -> Self {
        Self::with_client(HttpTransferClient::new())
    }

    pub fn with_client(http: HttpTransferClient) -> Self {
        Self {
            http,
            driver: Arc::new(Driver::start()),
        }
    }

    /// The asynchronous client this one drives.
    pub fn http(&self) -> &HttpTransferClient {
        &self.http
    }

    /// Runs `future` to completion on the calling thread, with its IO driven by the client's
    /// runtime.
    pub fn block_on<F: Future>(&self, future: F) -> F::Output {
        self.driver.handle.block_on(future)
    }
}

impl Default for ReqwestTransferClient {
    fn default() -> Self {
        Self::new()
    }
}

impl ReqwestTransferClient {
    /// Uploads `size_bytes` from `reader` to `url`.
    pub fn put_reader<R: Read + Send + 'static>(
        &self,
        url: &str,
        reader: R,
        size_bytes: u64,
    ) -> Result<(), TransferError> {
        self.block_on(self.http.put(url, reader_stream(reader), size_bytes))
    }

    /// Downloads `url` as a reader.
    ///
    /// `expected_size_bytes` is the size declared by the manifest, `None` when none was. It
    /// selects the transfer strategy and the deadline. A failure to open the download is
    /// reported here rather than on the first read.
    pub fn get_reader(
        &self,
        url: &str,
        expected_size_bytes: Option<u64>,
    ) -> Result<Box<dyn Read + Send>, TransferError> {
        let body = self.block_on(self.http.get(url, expected_size_bytes))?;
        let chunks = Streaming::spawn(&*self.driver, 4, |sink| pump(body, sink));

        Ok(Box::new(ByteReader {
            chunks: chunks.blocking_iter(),
            current: Bytes::new(),
            _driver: Arc::clone(&self.driver),
        }))
    }
}

async fn pump(mut body: ByteStream, sink: StreamingSink<Bytes, TransferError>) {
    while let Some(item) = body.next().await {
        match item {
            Ok(chunk) => {
                if sink.send(chunk).await.is_err() {
                    return;
                }
            }
            Err(error) => {
                sink.fail(error).await;
                return;
            }
        }
    }
}

/// A single-threaded runtime on a thread of its own, alive as long as anyone holds it.
struct Driver {
    handle: Handle,
    shutdown: Option<oneshot::Sender<()>>,
}

impl Driver {
    fn start() -> Self {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build the transfer runtime");
        let handle = runtime.handle().clone();
        let (shutdown, stopped) = oneshot::channel::<()>();
        thread::Builder::new()
            .name("tracel-transfer".to_string())
            .spawn(move || {
                runtime.block_on(async {
                    let _ = stopped.await;
                });
            })
            .expect("failed to start the transfer runtime thread");

        Self {
            handle,
            shutdown: Some(shutdown),
        }
    }
}

impl Drop for Driver {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}

impl Spawn for Driver {
    fn spawn(&self, future: SpawnedFuture) {
        self.handle.spawn(future);
    }
}

/// Reads a download chunk by chunk, keeping its runtime alive until it is dropped.
struct ByteReader {
    chunks: BlockingIter<Bytes, TransferError>,
    current: Bytes,
    _driver: Arc<Driver>,
}

impl Read for ByteReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        while self.current.is_empty() {
            match self.chunks.next() {
                Some(Ok(chunk)) => self.current = chunk,
                Some(Err(error)) => return Err(io::Error::other(error)),
                None => return Ok(0),
            }
        }
        let read = buf.len().min(self.current.len());
        buf[..read].copy_from_slice(&self.current.split_to(read));
        Ok(read)
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_server::TestServer;
    use super::*;

    fn file(len: usize) -> Vec<u8> {
        (0..len).map(|byte| (byte % 251) as u8).collect()
    }

    #[test]
    fn a_blocking_download_needs_no_runtime_on_the_calling_thread() {
        let server = TestServer::serve(file(70 * 1024), true);
        let client = ReqwestTransferClient::new();

        let mut reader = client
            .get_reader(&server.url("/file"), Some(70 * 1024))
            .unwrap();
        let mut read = Vec::new();
        reader.read_to_end(&mut read).unwrap();

        assert_eq!(read, file(70 * 1024));
    }

    #[test]
    fn a_reader_outlives_the_client_that_opened_it() {
        let server = TestServer::serve(file(2048), true);
        let client = ReqwestTransferClient::new();
        let mut reader = client.get_reader(&server.url("/file"), Some(2048)).unwrap();

        drop(client);
        let mut read = Vec::new();
        reader.read_to_end(&mut read).unwrap();

        assert_eq!(read, file(2048));
    }

    #[test]
    fn a_missing_file_fails_when_opened_rather_than_when_read() {
        let server = TestServer::serve(file(10), true);
        let client = ReqwestTransferClient::new();

        let error = client
            .get_reader(&server.url("/missing"), Some(10))
            .err()
            .expect("404 is a transport failure");

        assert!(error.to_string().contains("404"), "{error}");
    }

    #[test]
    fn a_blocking_upload_delivers_the_reader_with_its_length() {
        let server = TestServer::serve(Vec::new(), true);
        let client = ReqwestTransferClient::new();
        let payload = file(3000);

        client
            .put_reader(
                &server.url("/upload"),
                io::Cursor::new(payload.clone()),
                3000,
            )
            .unwrap();

        let received = server.received();
        assert_eq!(received[0].header("content-length"), Some("3000"));
        assert_eq!(received[0].body, payload);
    }
}
