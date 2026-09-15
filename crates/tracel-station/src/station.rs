use std::fmt;
use std::sync::Arc;

use tracel_artifact::HttpTransferClient;
use tracel_client::station::StationClient;
use tracel_datasets::Datasets;
use tracel_experiment::ExperimentModule;
use tracel_models::Models;
use tracel_task::{Spawn, TokioRuntime};
use url::Url;

/// A client rooted at one Station URL.
///
/// The connection owns the runtime its work runs on; nothing a caller does requires one of the
/// caller's own.
#[derive(Clone)]
pub struct Station {
    inner: Arc<StationInner>,
}

pub struct StationInner {
    pub client: StationClient,
    /// The executor behind [`spawn`](Self::spawn), for the ports whose contract is synchronous:
    /// each one `block_on`s a client call from the caller's thread, never from a task on the
    /// runtime itself, which would stall it.
    // Every such bridge goes away when its capability hands back tasks of its own.
    pub runtime: Arc<TokioRuntime>,
    pub spawn: Arc<dyn Spawn>,
    pub transfer: HttpTransferClient,
}

impl Station {
    /// Binds to a Station without performing I/O.
    pub fn connect(url: Url) -> Self {
        let runtime = Arc::new(TokioRuntime::start().expect("failed to start the station runtime"));
        let spawn: Arc<dyn Spawn> = runtime.clone();
        let transfer = HttpTransferClient::new(Arc::clone(&spawn));

        Self {
            inner: Arc::new(StationInner {
                client: StationClient::from_url(url),
                runtime,
                spawn,
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
        Models::new(
            Arc::new(crate::models::StationModelOps {
                station: Arc::clone(&self.inner),
            }),
            Arc::clone(&self.inner.spawn),
        )
    }
}

impl fmt::Debug for Station {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Station").finish_non_exhaustive()
    }
}
