pub mod cli;

pub mod crate_gen;
pub mod logging;
pub mod registry;

#[cfg(feature = "fail")]
compile_error!("fail feature is not supported in heat-sdk-cli");