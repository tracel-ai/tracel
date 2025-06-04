// #![cfg_attr(not(feature = "std"), no_std)]
#![warn(missing_docs)]

//! # Tracel

/// Heat SDK
#[cfg(feature = "client")]
pub use burn_central_client::*;

/// Heat macros
#[cfg(feature = "client")]
pub mod macros {
    pub use burn_central_cli_macros::heat;
    pub use burn_central_cli_macros::heat_cli_main;
}

/// Heat SDK CLI
#[cfg(feature = "cli")]
pub mod cli {
    pub use burn_central_cli::*;
}
