#![deny(missing_docs)]

//! Burn-free, blocking SDK for the Tracel console domain.
//!
//! [`Console`] owns one normalized console URL and authentication state. Project handles are cheap
//! views over that shared client and vend backend-agnostic model capabilities without performing
//! I/O when created.

mod console;
mod domain;
mod error;

pub mod auth;

pub use console::{Auth, Console, ProjectHandle, SessionToken};
pub use domain::{Namespace, NamespaceKind, Organization, Project, User, UserSummary, Visibility};
pub use error::ConsoleError;
pub use tracel_models::{
    BundleSink, DownloadObserver, Model, ModelVersion, Models, ModelsError, Page, VersionFile,
    VersionId, VersionManifest,
};

use url::Url;

fn normalize_base_url(url: &str) -> Result<Url, ConsoleError> {
    let mut url = Url::parse(url).map_err(|error| ConsoleError::InvalidUrl(error.to_string()))?;
    if !matches!(url.scheme(), "http" | "https") || url.cannot_be_a_base() {
        return Err(ConsoleError::InvalidUrl(
            "expected an absolute HTTP or HTTPS URL".to_string(),
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(ConsoleError::InvalidUrl(
            "base URLs cannot contain a query or fragment".to_string(),
        ));
    }
    if !url.path().ends_with('/') {
        url.set_path(&format!("{}/", url.path()));
    }
    Ok(url)
}
