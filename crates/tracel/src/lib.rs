// #![cfg_attr(not(feature = "std"), no_std)]
#![warn(missing_docs)]

//! # Tracel

/// Burn Central Client
#[cfg(feature = "client")]
pub use burn_central_client::*;

/// Burn Central macros
#[cfg(feature = "client")]
pub mod macros {
    pub use burn_central_cli_macros::burn;
    pub use burn_central_cli_macros::burn_central_main;
}

/// Burn Central CLI
#[cfg(feature = "cli")]
pub mod cli {
    pub use burn_central_cli::*;
}
