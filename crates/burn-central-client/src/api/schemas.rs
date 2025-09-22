//! API schemas for Burn Central client
//!
//! This module contains all the request and response schemas used for
//! communicating with the Burn Central API.
//!
//! # Organization
//!
//! - [`request`] - Schemas for data sent to the API
//! - [`response`] - Schemas for data received from the API
//!
//! Common types are re-exported at the module level for convenience.

pub mod request;
pub mod response;

pub use request::*;
pub use response::*;
