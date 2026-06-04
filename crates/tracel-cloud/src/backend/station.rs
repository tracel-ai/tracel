use burn_central_client::StationClient;
use tracel_experiment::ExperimentRun;
use url::Url;

use tracel_experiment::error::ExperimentError;

use crate::{Backend, Context};

#[derive(Debug, Clone)]
pub struct StationBackend {
    pub client: StationClient,
}

impl StationBackend {
    pub fn create_context(url: Url) -> Context {
        let backend = Backend::Station(StationBackend {
            client: StationClient::from_url(url),
        });
        Context::new(backend)
    }

    pub fn setup_experiment(&self, routine: String) -> Result<ExperimentRun, ExperimentError> {
        ExperimentRun::station(self.client.clone(), routine)
    }
}
