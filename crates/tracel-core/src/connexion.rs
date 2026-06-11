use std::path::PathBuf;
use std::sync::Arc;

#[cfg(feature = "station")]
use url::Url;

use crate::backend::cloud::{CloudBackend, CloudError};
use crate::backend::local::LocalBackend;
#[cfg(feature = "station")]
use crate::backend::station::StationBackend;
use tracel_experiment::ExperimentProvider;

#[derive(Debug, Clone)]
pub enum Connexion {
    Cloud,
    None(PathBuf),
    #[cfg(feature = "station")]
    Station(Url),
}

impl Connexion {
    pub(crate) fn into_provider(self) -> Result<Arc<dyn ExperimentProvider>, ContextError> {
        match self {
            Connexion::Cloud => Ok(Arc::new(CloudBackend::create_context()?)),
            Connexion::None(path) => Ok(Arc::new(LocalBackend::create_context(path))),
            #[cfg(feature = "station")]
            Connexion::Station(url) => Ok(Arc::new(StationBackend::create_context(url))),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ContextError {
    #[error(transparent)]
    Cloud(#[from] CloudError),
}
