#![deny(missing_docs)]

//! Burn-free, blocking SDK for the Tracel console domain.
//!
//! [`Console`] owns one connection to the console. Creating it performs no I/O: the first
//! request verifies the credential. Project handles are cheap views over that shared client and
//! vend backend-agnostic, project-scoped capabilities without performing I/O when created.
//!
//! [`Console::from_env`] and [`ProjectHandle::from_env`] read the console, the credential and the
//! project from `TRACEL_ENV`, `TRACEL_API_KEY`, `TRACEL_NAMESPACE` and `TRACEL_PROJECT`, falling
//! back to the `tracel login` sign-in and to `tracel.toml`.

mod console;
mod datasets;
mod domain;
mod env;
mod error;
mod experiment;
mod inference;
mod login;
mod models;
mod wire;

pub use console::{Console, ProjectHandle};
pub use domain::{Namespace, NamespaceKind, Organization, Project, User, Visibility};
pub use error::ConsoleError;
pub use login::{DeviceApproval, DeviceLogin, refresh_session, sign_out};
pub use tracel_client::console::auth::IssuedAppSession;
pub use tracel_client::console::{AccessToken, Env, RefreshToken, TracelCredentials};
