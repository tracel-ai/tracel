#[allow(clippy::module_inception)]
pub mod remote;
pub use remote::*;

pub mod inference;
pub mod training;
