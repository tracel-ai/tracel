// #![cfg_attr(not(feature = "std"), no_std)]
#![warn(missing_docs)]

//! # Tracel

/// Heat SDK
pub mod heat {
    #[cfg(feature = "heat-sdk")]
    pub use heat_sdk::*;

    /// Heat macros
    #[cfg(feature = "heat-sdk")]
    pub mod macros {
        pub use heat_sdk_cli_macros::heat;
        pub use heat_sdk_cli_macros::heat_cli_main;
    }

    /// Heat SDK CLI
    #[cfg(feature = "heat-sdk-cli")]
    pub mod cli {
        pub use heat_sdk_cli::*;
    }
}
