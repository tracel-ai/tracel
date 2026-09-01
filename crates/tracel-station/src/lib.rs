#![deny(missing_docs)]

//! Burn-free, blocking client for a Tracel Station.
//!
//! [`Station`] owns one connection and vends backend-agnostic capabilities without performing
//! I/O when created.

mod datasets;
mod error;
mod models;
mod station;
mod wire;

pub use error::StationError;
pub use station::Station;
