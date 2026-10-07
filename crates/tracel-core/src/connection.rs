use std::path::PathBuf;
use std::sync::Arc;

#[cfg(feature = "station")]
use url::Url;

use tracel_console::{ConsoleError, ProjectHandle};

use crate::backend::Backend;
use crate::backend::local::LocalBackend;
#[cfg(feature = "station")]
use tracel_station::Station;

#[derive(Debug, Clone)]
pub enum Connection {
    Cloud,
    Offline(PathBuf),
    #[cfg(feature = "station")]
    Station(Url),
}

/// The backend `connection` reaches, bound without performing network I/O.
pub fn backend(connection: Connection) -> Result<Arc<dyn Backend>, ContextError> {
    match connection {
        Connection::Cloud => Ok(Arc::new(ProjectHandle::from_env()?)),
        Connection::Offline(path) => Ok(Arc::new(LocalBackend::new(path))),
        #[cfg(feature = "station")]
        Connection::Station(url) => Ok(Arc::new(Station::connect(url))),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ContextError {
    #[error(transparent)]
    Console(#[from] ConsoleError),
}
