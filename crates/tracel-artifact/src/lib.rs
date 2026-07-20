//! This crate centralizes traits, structures and utilities for handling artifacts.

mod tools;
mod transfer;

pub mod bundle;
pub mod download;
pub mod upload;

pub use tools::validation::normalize_checksum;
pub use transfer::{FileTransferClient, TransferError};
#[cfg(feature = "transfer")]
pub use transfer::ReqwestTransferClient;
