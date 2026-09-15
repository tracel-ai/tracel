//! This crate centralizes traits, structures and utilities for handling artifacts.

mod tools;
mod transfer;

pub mod bundle;
pub mod download;
pub mod upload;

pub use tools::validation::normalize_checksum;
#[cfg(not(target_arch = "wasm32"))]
pub use transfer::ReqwestTransferClient;
pub use transfer::{
    ByteStream, HttpTransferClient, TransferClient, TransferError, TransferObserver, reader_stream,
};
