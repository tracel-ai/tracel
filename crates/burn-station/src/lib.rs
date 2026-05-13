//! Crate for exposing the core station capabilities.

mod station;

pub mod dataset;

pub use station::Station;

#[doc(inline)]
pub use burn_central_experiment as experiment;

#[doc(inline)]
pub use burn_central_artifact as artifact;
