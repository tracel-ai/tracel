#![deny(missing_docs)]

//! Burn-free, blocking SDK for the Tracel console domain.
//!
//! [`Console`] owns one connection to the console. Project handles are cheap
//! views over that shared client and vend backend-agnostic model capabilities without performing
//! I/O when created.

mod console;
mod datasets;
mod domain;
mod error;
mod login;

pub use console::{Console, ProjectHandle};
pub use domain::{Namespace, NamespaceKind, Organization, Project, User, Visibility};
pub use error::ConsoleError;
pub use login::{DeviceApproval, DeviceLogin};
pub use tracel_client::console::{Env, SessionToken, TracelCredentials};
