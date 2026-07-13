//! Shared setup for the Tracel examples.

use tracel::{Connection, Context};

/// Builds the SDK [`Context`] the examples run against.
///
/// The backend is chosen at runtime from `TRACEL_CONNECTION`, so an example ships telemetry
/// locally or to the cloud without editing code:
///
/// - unset or `offline`: record locally under `./runs`, no account required
/// - `cloud`: ship to the [console](https://console.tracel.ai) (needs `tracel login`)
///
/// This is the pattern to copy into a real application: resolve the [`Connection`] once, from the
/// environment or your own config, then share the [`Context`] across the program.
pub fn context() -> anyhow::Result<Context> {
    Ok(Context::new(connection()?)?)
}

fn connection() -> anyhow::Result<Connection> {
    match std::env::var("TRACEL_CONNECTION").as_deref() {
        Err(_) | Ok("offline") => Ok(Connection::Offline("./runs".into())),
        Ok("cloud") => Ok(Connection::Cloud),
        Ok(other) => {
            anyhow::bail!("unknown TRACEL_CONNECTION={other:?}; expected `offline` or `cloud`")
        }
    }
}
