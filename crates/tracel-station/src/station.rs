use std::fmt;
use std::future::Future;
use std::sync::Arc;

use futures::Stream;
use tracel_artifact::HttpTransferClient;
use tracel_client::station::StationClient;
use tracel_datasets::Datasets;
use tracel_experiment::ExperimentModule;
use tracel_models::Models;
use tracel_task::{Job, MaybeSend, Runtime, Streaming};
use url::Url;

/// A client rooted at one Station URL.
///
/// Every operation is a [`Job`] the caller drives where they choose. The transport's runtime is
/// the connection's business: it borrows the tokio runtime the caller connected from, or starts
/// one of its own, and attaches each call to it so that any executor can drive the result.
#[derive(Clone)]
pub struct Station {
    inner: Arc<StationInner>,
}

pub struct StationInner {
    pub client: StationClient,
    /// Drives the transport's IO and runs the connection's actors.
    pub runtime: Arc<Runtime>,
    pub transfer: HttpTransferClient,
}

impl StationInner {
    /// Hands `call` back as a job any executor can drive, its IO driven by the runtime.
    pub fn attach<T, E, F>(&self, call: F) -> Job<T, E>
    where
        F: Future<Output = Result<T, E>> + MaybeSend + 'static,
    {
        Job::new(self.runtime.attach(call))
    }

    /// Hands `stream` back as items any executor can pull, its IO driven by the runtime.
    pub fn attach_stream<T, E, S>(&self, stream: S) -> Streaming<T, E>
    where
        S: Stream<Item = Result<T, E>> + MaybeSend + 'static,
    {
        Streaming::new(self.runtime.attach_stream(stream))
    }
}

impl Station {
    /// Binds to a Station without performing I/O.
    ///
    /// The runtime is the tokio runtime the caller is inside, or one of the connection's own.
    pub fn connect(url: Url) -> Self {
        let runtime = Arc::new(Runtime::acquire().expect("failed to start the station runtime"));
        let transfer = HttpTransferClient::new();

        Self {
            inner: Arc::new(StationInner {
                client: StationClient::from_url(url),
                runtime,
                transfer,
            }),
        }
    }

    /// Returns experiment operations scoped to this Station without performing I/O.
    pub fn experiments(&self) -> ExperimentModule {
        ExperimentModule::new(Arc::new(crate::experiment::StationExperimentProvider {
            station: Arc::clone(&self.inner),
        }))
    }

    /// Returns dataset operations scoped to this Station without performing I/O.
    pub fn datasets(&self) -> Datasets {
        Datasets::new(Arc::new(crate::datasets::StationDatasetOps {
            station: Arc::clone(&self.inner),
        }))
    }

    /// Returns model operations scoped to this Station without performing I/O.
    pub fn models(&self) -> Models {
        Models::new(Arc::new(crate::models::StationModelOps {
            station: Arc::clone(&self.inner),
        }))
    }
}

impl fmt::Debug for Station {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Station").finish_non_exhaustive()
    }
}
