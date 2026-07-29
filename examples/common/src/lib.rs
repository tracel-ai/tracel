//! Shared setup for the Tracel examples.

use tracel::{Connection, Context};
use url::Url;

/// Builds the SDK [`Context`] the examples run against.
///
/// The backend is chosen at runtime from `TRACEL_CONNECTION`, so an example ships telemetry
/// locally, to the cloud, or to a Tracel Station without editing code:
///
/// - unset or `offline`: record locally under `./runs`, no account required
/// - `cloud`: ship to the [console](https://console.tracel.ai) (needs `tracel login`)
/// - `station`: ship to the Tracel Station at [`station_url`]
///
/// This is the pattern to copy into a real application: resolve the [`Connection`] once, from the
/// environment or your own config, then share the [`Context`] across the program.
pub fn context() -> anyhow::Result<Context> {
    Ok(Context::new(connection()?)?)
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

fn connection() -> anyhow::Result<Connection> {
    match std::env::var("TRACEL_CONNECTION").as_deref() {
        Err(_) | Ok("offline") => Ok(Connection::Offline("./runs".into())),
        Ok("cloud") => Ok(Connection::Cloud),
        Ok("station") => Ok(Connection::Station(station_url()?)),
        Ok(other) => {
            anyhow::bail!(
                "unknown TRACEL_CONNECTION={other:?}; expected `offline`, `cloud`, or `station`"
            )
        }
    }
}
