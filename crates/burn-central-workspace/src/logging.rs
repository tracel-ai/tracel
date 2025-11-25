//! Logging utilities for the Burn Central library
//!
//! This module provides logging macros that can be used throughout the library.
//! Unlike CLI-specific printing, these use the standard `log` crate for output.

pub use log::{debug, error, info, warn};

/// Print an informational message
#[macro_export]
macro_rules! print_info {
    ($($arg:tt)*) => {
        log::info!($($arg)*);
    };
}

/// Print a warning message
#[macro_export]
macro_rules! print_warn {
    ($($arg:tt)*) => {
        log::warn!($($arg)*);
    };
}

/// Print an error message
#[macro_export]
macro_rules! print_err {
    ($($arg:tt)*) => {
        log::error!($($arg)*);
    };
}

/// Print a debug message
#[macro_export]
macro_rules! print_debug {
    ($($arg:tt)*) => {
        log::debug!($($arg)*);
    };
}
