use std::path::PathBuf;
use std::sync::Arc;

#[cfg(feature = "station")]
use url::Url;

use tracel_console::{Console, ConsoleError};

use crate::backend::Backend;
use crate::backend::local::LocalBackend;
use crate::cloud::CloudError;
#[cfg(feature = "station")]
use tracel_station::Station;

#[derive(Debug, Clone)]
pub enum Connection {
    Cloud,
    Offline(PathBuf),
    #[cfg(feature = "station")]
    Station(Url),
}

impl Connection {
    pub(crate) fn into_backend(self) -> Result<Arc<dyn Backend>, ContextError> {
        match self {
            Connection::Cloud => {
                let env = crate::cloud::discover_env()?;
                let credentials = crate::cloud::discover_credentials(&env)?;
                let (namespace, project) = crate::cloud::discover_namespace_project()?;

                let console = Console::connect(env, &credentials)?;
                let project = console.project(namespace, project);

                Ok(Arc::new(project))
            }
            Connection::Offline(path) => Ok(Arc::new(LocalBackend::create_context(path))),
            #[cfg(feature = "station")]
            Connection::Station(url) => Ok(Arc::new(Station::connect(url))),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ContextError {
    #[error(transparent)]
    Cloud(#[from] CloudError),
    #[error(transparent)]
    Console(#[from] ConsoleError),
}
