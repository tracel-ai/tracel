use std::fmt;
use std::sync::Arc;

use tracel_artifact::ReqwestTransferClient;
use tracel_client::station::StationClient;
use tracel_datasets::Datasets;
use tracel_experiment::ExperimentModule;
use tracel_models::Models;
use url::Url;

/// A blocking client rooted at one Station URL.
#[derive(Clone)]
pub struct Station {
    inner: Arc<StationInner>,
}

pub struct StationInner {
    pub client: StationClient,
    pub transfer_client: ReqwestTransferClient,
}

impl Station {
    /// Binds to a Station without performing I/O.
    pub fn connect(url: Url) -> Self {
        Self {
            inner: Arc::new(StationInner {
                client: StationClient::from_url(url),
                transfer_client: ReqwestTransferClient::new(),
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
