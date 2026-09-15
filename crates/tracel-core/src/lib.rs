mod backend;
// Discovery reads the environment and the filesystem; a browser is told instead.
#[cfg(not(target_arch = "wasm32"))]
mod cloud;
mod connection;
mod context;

pub mod experiment;
pub mod inference;

pub use connection::{Connection, ContextError};
pub use context::Context;
