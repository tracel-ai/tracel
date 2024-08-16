#[allow(clippy::module_inception)]
pub mod local;
pub use local::*;

pub mod inference;
pub mod training;
