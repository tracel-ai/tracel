//! Shared setup for the Tracel examples.

use std::sync::Arc;

use tracel::experiment::ExperimentModule;
use tracel::inference::InferenceModule;
use tracel::{
    CloudBackend, CloudInferenceProvider, DefaultInferenceProvider, LocalBackend, StationBackend,
};
use url::Url;

/// Builds the experiment and inference modules the examples run against.
///
/// The backend is chosen at runtime from `TRACEL_CONNECTION`, so an example ships telemetry
/// locally, to the cloud, or to a Tracel Station without editing code:
///
/// - unset or `offline`: record locally under `./runs`, no account required
/// - `cloud`: ship to the [console](https://console.tracel.ai) (needs `tracel login`)
/// - `station`: ship to the Tracel Station at [`station_url`]
///
/// This is the pattern to copy into a real application: construct the concrete backend once, then
/// share it with the modules that use it.
pub fn modules() -> anyhow::Result<(ExperimentModule, InferenceModule)> {
    match std::env::var("TRACEL_CONNECTION").as_deref() {
        Err(_) | Ok("offline") => {
            let backend = Arc::new(LocalBackend::new("./runs"));
            let experiment = ExperimentModule::new(move |name, attributes| {
                backend.create_experiment(name, attributes)
            });
            let backend = Arc::new(DefaultInferenceProvider::new());
            let inference = InferenceModule::new(move |name| backend.create_session(name));
            Ok((experiment, inference))
        }
        Ok("cloud") => {
            let backend = Arc::new(CloudBackend::discover()?);
            let inference_backend = Arc::new(CloudInferenceProvider::new(
                backend.client().clone(),
                backend.namespace().to_string(),
                backend.project().to_string(),
            ));
            let experiment = ExperimentModule::new(move |name, attributes| {
                backend.create_experiment(name, attributes)
            });
            let inference =
                InferenceModule::new(move |name| inference_backend.create_session(name));
            Ok((experiment, inference))
        }
        Ok("station") => {
            let backend = Arc::new(StationBackend::new(station_url()?)?);
            let experiment = ExperimentModule::new(move |name, attributes| {
                backend.create_experiment(name, attributes)
            });
            let backend = Arc::new(DefaultInferenceProvider::new());
            let inference = InferenceModule::new(move |name| backend.create_session(name));
            Ok((experiment, inference))
        }
        Ok(other) => {
            anyhow::bail!(
                "unknown TRACEL_CONNECTION={other:?}; expected `offline`, `cloud`, or `station`"
            )
        }
    }
}

/// The Tracel Station base URL, from `TRACEL_STATION_URL` (default `http://localhost:8000`).
///
/// The same URL serves both roles: the experiment backend of the `station` connection, and the
/// queue a runner example registers with.
pub fn station_url() -> anyhow::Result<Url> {
    let url =
        std::env::var("TRACEL_STATION_URL").unwrap_or_else(|_| "http://localhost:8000".to_string());
    Ok(Url::parse(&url)?)
}
