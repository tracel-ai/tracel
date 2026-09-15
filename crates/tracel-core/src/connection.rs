use std::path::PathBuf;
use std::sync::Arc;

#[cfg(feature = "station")]
use url::Url;

use tracel_console::{Console, ConsoleError, TracelCredentials};
use tracel_task::Job;

use crate::backend::Backend;
use crate::backend::local::LocalBackend;
#[cfg(not(target_arch = "wasm32"))]
use crate::cloud::CloudError;
#[cfg(feature = "station")]
use tracel_station::Station;

/// Where a [`Context`](crate::Context) sends its work.
#[derive(Debug, Clone)]
pub enum Connection {
    /// The console, with credentials and project discovered from the environment, the
    /// credentials file, and `tracel.toml`.
    #[cfg(not(target_arch = "wasm32"))]
    Cloud,
    /// The console, told everything.
    Console {
        credentials: TracelCredentials,
        namespace: String,
        project: String,
    },
    Offline(PathBuf),
    #[cfg(feature = "station")]
    Station(Url),
}

impl Connection {
    pub(crate) fn into_backend(self) -> Job<Arc<dyn Backend>, ContextError> {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Connection::Cloud => {
                let discovered = crate::cloud::discover_credentials().and_then(|credentials| {
                    let (namespace, project) = crate::cloud::discover_namespace_project()?;
                    Ok(Connection::Console {
                        credentials,
                        namespace,
                        project,
                    })
                });
                match discovered {
                    Ok(connection) => connection.into_backend(),
                    Err(error) => Job::failed(error.into()),
                }
            }
            Connection::Console {
                credentials,
                namespace,
                project,
            } => {
                let console = Console::connect(&credentials);
                Job::new(async move {
                    let project = console.await?.project(namespace, project);
                    Ok(Arc::new(project) as Arc<dyn Backend>)
                })
            }
            Connection::Offline(path) => Job::ready(Arc::new(LocalBackend::new(path))),
            #[cfg(feature = "station")]
            Connection::Station(url) => Job::ready(Arc::new(Station::connect(url))),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ContextError {
    #[cfg(not(target_arch = "wasm32"))]
    #[error(transparent)]
    Cloud(#[from] CloudError),
    #[error(transparent)]
    Console(#[from] ConsoleError),
}
