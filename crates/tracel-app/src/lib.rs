//! App front-ends for running Tracel capabilities (experiments, inference, ...).
//!
//! Each front-end owns its own way of registering and driving capabilities, expressed through a
//! front-end-local trait that capability adapters implement:
//! - [`cli`] runs a named capability from a string config, via [`cli::CliCommand`].
//! - [`server`] serves a named capability over HTTP, via `server::ServerRoute`.
//!
//! There is intentionally no shared job registry across front-ends: a CLI and an HTTP server have
//! different requirements (string parsing vs. request bodies, stdout vs. streamed responses), so
//! each interprets a capability in its own terms.

/// Command-line front-end.
pub mod cli;
/// HTTP server front-end.
#[cfg(feature = "server")]
pub mod server;
