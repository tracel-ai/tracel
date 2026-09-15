#![deny(missing_docs)]

//! Burn-free SDK for the Tracel console domain.
//!
//! [`Console`] owns one connection to the console. Project handles are cheap
//! views over that shared client and vend backend-agnostic, project-scoped capabilities without
//! performing I/O when created.

mod console;
mod domain;
mod env;
mod error;
mod models;
mod wire;

// Capabilities not yet handed back as jobs still bridge into a runtime of their own.
#[cfg(not(target_arch = "wasm32"))]
mod datasets;
#[cfg(not(target_arch = "wasm32"))]
mod experiment;
#[cfg(not(target_arch = "wasm32"))]
mod inference;
#[cfg(not(target_arch = "wasm32"))]
mod login;

pub use console::{Console, ProjectHandle};
pub use domain::{Namespace, NamespaceKind, Organization, Project, User, Visibility};
pub use error::ConsoleError;
#[cfg(not(target_arch = "wasm32"))]
pub use login::{DeviceApproval, DeviceLogin};
pub use tracel_client::console::{SessionToken, TracelCredentials};
