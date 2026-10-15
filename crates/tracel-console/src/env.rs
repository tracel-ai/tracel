//! Reading which console to reach, with which credential, and which project, from the
//! environment and `tracel.toml`.

use std::ffi::OsString;
use std::path::Path;
use std::sync::Arc;

use serde::Deserialize;
use tracel_client::ClientError;
use tracel_client::console::auth::DeviceAuthClient;
use tracel_client::console::{
    AppSession, Env, FileSessionStore, SessionStore, SessionStoreError, TracelCredentials,
};
use url::Url;

use crate::ConsoleError;

const TRACEL_ENV: &str = "TRACEL_ENV";
const TRACEL_API_KEY: &str = "TRACEL_API_KEY";
const TRACEL_NAMESPACE: &str = "TRACEL_NAMESPACE";
const TRACEL_PROJECT: &str = "TRACEL_PROJECT";
const TRACEL_TOML: &str = "tracel.toml";

/// The client `tracel login` signs in as; renewing its session must name it too.
const TRACEL_CLI_CLIENT_ID: &str = "tracel-cli";

/// The values `TRACEL_ENV` accepts, as an error lists them.
const TRACEL_ENV_VALUES: &str =
    "Production, Development, Staging(n) or staging-n, with n from 0 to 255";

/// The console `TRACEL_ENV` names, `Production` when unset or empty.
///
/// Accepts `Production`, `Development` and `Staging(n)`, and the `production`, `development` and
/// `staging-n` spellings of the `tracel` CLI. Fails with [`ConsoleError::InvalidSetting`] for any
/// other value.
pub fn env_from_environment() -> Result<Env, ConsoleError> {
    env_from_vars(|name| std::env::var_os(name))
}

/// [`env_from_environment`], reading `TRACEL_ENV` from `lookup`.
pub fn env_from_vars(lookup: impl Fn(&str) -> Option<OsString>) -> Result<Env, ConsoleError> {
    let value = lookup(TRACEL_ENV).map(|value| value.to_string_lossy().into_owned());
    parse_env(value.as_deref())
}

/// Reads the console a `TRACEL_ENV` value names, in the spellings the `tracel` CLI accepts.
fn parse_env(value: Option<&str>) -> Result<Env, ConsoleError> {
    match value {
        None | Some("" | "Production" | "production") => Ok(Env::Production),
        Some("Development" | "development") => Ok(Env::Development),
        Some(value) => value
            .strip_prefix("Staging(")
            .and_then(|number| number.strip_suffix(')'))
            .or_else(|| value.strip_prefix("staging-"))
            .filter(|number| number.bytes().all(|byte| byte.is_ascii_digit()))
            .and_then(|number| number.parse::<u8>().ok())
            .map(Env::Staging)
            .ok_or_else(|| ConsoleError::InvalidSetting {
                variable: TRACEL_ENV,
                value: value.to_string(),
                expected: TRACEL_ENV_VALUES,
            }),
    }
}

/// Where a console connection takes its credential from.
#[derive(Debug, Clone)]
pub enum CredentialSource {
    /// This credential, such as an API key or the token of a job.
    Explicit(TracelCredentials),
    /// The session `tracel login` stored for the console, which the client renews as the run
    /// goes.
    StoredLogin,
}

impl CredentialSource {
    /// `TRACEL_API_KEY` when it is set and not empty, else the stored `tracel login` session.
    pub fn from_env() -> Self {
        Self::from_vars(|name| std::env::var_os(name))
    }

    /// [`from_env`](Self::from_env), reading `TRACEL_API_KEY` from `lookup`.
    pub fn from_vars(lookup: impl Fn(&str) -> Option<OsString>) -> Self {
        non_empty(lookup(TRACEL_API_KEY)).map_or(Self::StoredLogin, |key| {
            Self::Explicit(TracelCredentials::api_key(key))
        })
    }

    /// The credential this source names for `env`'s console.
    ///
    /// Reads a stored session from disk, without network I/O. Fails with
    /// [`ConsoleError::NoCredentials`] when `tracel login` stored none for that console.
    pub fn resolve(&self, env: &Env) -> Result<TracelCredentials, ConsoleError> {
        self.resolve_with(env, FileSessionStore::for_server)
    }

    /// [`resolve`](Self::resolve), reading a stored session from the store `sessions` gives for
    /// the console's URL. Only [`StoredLogin`](Self::StoredLogin) asks `sessions` for it.
    pub fn resolve_with(
        &self,
        env: &Env,
        sessions: impl FnOnce(&Url) -> Result<FileSessionStore, SessionStoreError>,
    ) -> Result<TracelCredentials, ConsoleError> {
        match self {
            Self::Explicit(credentials) => Ok(credentials.clone()),
            Self::StoredLogin => {
                let store = sessions(&env.get_url()).map_err(ClientError::from)?;
                stored_login(env, store)
            }
        }
    }
}

/// The app session `tracel login` stored in `store` for `env`'s console.
fn stored_login(env: &Env, store: FileSessionStore) -> Result<TracelCredentials, ConsoleError> {
    if store.load().map_err(ClientError::from)?.is_none() {
        return Err(ConsoleError::NoCredentials);
    }

    let device_auth = DeviceAuthClient::new(env.clone(), TRACEL_CLI_CLIENT_ID);
    Ok(TracelCredentials::app_session(AppSession::new(
        Arc::new(store),
        device_auth,
    )))
}

/// A console project, named by its owner namespace and its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRef {
    /// The namespace that owns the project.
    pub namespace: String,
    /// The project's name.
    pub name: String,
}

impl ProjectRef {
    /// Names the project `name` that `namespace` owns.
    pub fn new(namespace: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            namespace: namespace.into(),
            name: name.into(),
        }
    }

    /// The project the environment names.
    ///
    /// The owner namespace is `TRACEL_NAMESPACE` and the project name `TRACEL_PROJECT`; either
    /// one unset is read from `namespace` or `project` in `tracel.toml` in the current
    /// directory. Fails with [`ConsoleError::NoNamespace`] or [`ConsoleError::NoProject`] when
    /// neither names it.
    pub fn from_env() -> Result<Self, ConsoleError> {
        Self::from_vars(|name| std::env::var_os(name), Path::new("."))
    }

    /// [`from_env`](Self::from_env), reading the variables from `lookup` and `tracel.toml` from
    /// `dir`.
    pub fn from_vars(
        lookup: impl Fn(&str) -> Option<OsString>,
        dir: &Path,
    ) -> Result<Self, ConsoleError> {
        let namespace = non_empty(lookup(TRACEL_NAMESPACE));
        let project = non_empty(lookup(TRACEL_PROJECT));
        let file = if namespace.is_some() && project.is_some() {
            TracelToml::default()
        } else {
            TracelToml::read(&dir.join(TRACEL_TOML))
        };

        let namespace = namespace
            .or(file.namespace)
            .ok_or(ConsoleError::NoNamespace)?;
        let name = project.or(file.project).ok_or(ConsoleError::NoProject)?;

        Ok(Self { namespace, name })
    }
}

/// A variable's value; an empty or non-UTF-8 one reads as unset.
fn non_empty(value: Option<OsString>) -> Option<String> {
    value
        .and_then(|value| value.into_string().ok())
        .filter(|value| !value.is_empty())
}

/// The project a `tracel.toml` names with its `namespace` and `project` keys.
#[derive(Deserialize, Default)]
struct TracelToml {
    namespace: Option<String>,
    project: Option<String>,
}

impl TracelToml {
    /// A missing, unreadable or malformed file names nothing.
    fn read(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .map(|contents| Self::parse(&contents))
            .unwrap_or_default()
    }

    fn parse(contents: &str) -> Self {
        toml::from_str(contents).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use rstest::{fixture, rstest};
    use tempfile::TempDir;
    use tracel_client::console::{AccessToken, RefreshToken, StoredAppSession};

    use super::*;

    const API_KEY: &str = "tcl_key";

    fn located(
        namespace: Option<&str>,
        project: Option<&str>,
        tracel_toml: &str,
    ) -> Result<ProjectRef, ConsoleError> {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("tracel.toml"), tracel_toml).unwrap();
        let lookup = |name: &str| match name {
            TRACEL_NAMESPACE => namespace.map(OsString::from),
            TRACEL_PROJECT => project.map(OsString::from),
            _ => None,
        };
        ProjectRef::from_vars(lookup, dir.path())
    }

    #[test]
    fn the_environment_names_the_project_over_tracel_toml() {
        let file = "namespace = \"file-owner\"\nproject = \"file-project\"";

        assert_eq!(
            located(Some("env-owner"), Some("env-project"), file).unwrap(),
            ProjectRef::new("env-owner", "env-project")
        );
    }

    #[test]
    fn tracel_toml_fills_in_what_the_environment_leaves_unset() {
        let file = "namespace = \"file-owner\"\nproject = \"file-project\"";

        assert_eq!(
            located(Some("env-owner"), None, file).unwrap(),
            ProjectRef::new("env-owner", "file-project")
        );
        assert_eq!(
            located(None, Some("env-project"), file).unwrap(),
            ProjectRef::new("file-owner", "env-project")
        );
        assert_eq!(
            located(None, None, file).unwrap(),
            ProjectRef::new("file-owner", "file-project")
        );
    }

    #[test]
    fn tracel_toml_names_the_project_only_by_namespace_and_project() {
        let file = "name = \"file-project\"\nowner = \"file-owner\"";

        let no_namespace = located(None, None, file).unwrap_err();
        let no_project = located(Some("env-owner"), None, file).unwrap_err();

        assert!(matches!(no_namespace, ConsoleError::NoNamespace));
        assert!(matches!(no_project, ConsoleError::NoProject));
        for error in [no_namespace, no_project] {
            assert!(
                error.to_string().contains("`namespace` and `project`"),
                "{error}"
            );
        }
    }

    #[test]
    fn a_malformed_tracel_toml_names_nothing() {
        assert!(matches!(
            located(None, Some("env-project"), "namespace = "),
            Err(ConsoleError::NoNamespace)
        ));
    }

    #[test]
    fn a_missing_namespace_or_project_says_where_to_set_it() {
        let no_namespace = located(None, Some("env-project"), "").unwrap_err();
        let no_project = located(Some("env-owner"), None, "").unwrap_err();

        assert!(matches!(no_namespace, ConsoleError::NoNamespace));
        assert!(no_namespace.to_string().contains("TRACEL_NAMESPACE"));
        assert!(no_namespace.to_string().contains("tracel.toml"));
        assert!(matches!(no_project, ConsoleError::NoProject));
        assert!(no_project.to_string().contains("TRACEL_PROJECT"));
        assert!(no_project.to_string().contains("tracel.toml"));
    }

    fn api_key(value: Option<&str>) -> CredentialSource {
        CredentialSource::from_vars(|name| match name {
            TRACEL_API_KEY => value.map(OsString::from),
            _ => None,
        })
    }

    #[fixture]
    fn sessions() -> TempDir {
        tempfile::tempdir().unwrap()
    }

    fn store_in(sessions: &TempDir) -> FileSessionStore {
        FileSessionStore::new(sessions.path().join("session.json"))
    }

    fn no_store(_: &Url) -> Result<FileSessionStore, SessionStoreError> {
        Err(SessionStoreError::new("no home directory"))
    }

    #[test]
    fn tracel_api_key_is_the_credential() {
        assert!(matches!(
            api_key(Some(API_KEY)),
            CredentialSource::Explicit(TracelCredentials::ApiKey(key)) if key == API_KEY
        ));
    }

    #[rstest]
    #[case::unset(None)]
    #[case::empty(Some(""))]
    fn without_tracel_api_key_the_credential_is_the_stored_login(#[case] value: Option<&str>) {
        assert!(matches!(api_key(value), CredentialSource::StoredLogin));
    }

    #[test]
    fn a_stored_login_is_read_from_the_store_for_its_console() {
        let env = Env::Staging(2);
        let mut asked_for = None;

        let _ = CredentialSource::StoredLogin.resolve_with(&env, |url| {
            asked_for = Some(url.clone());
            no_store(url)
        });

        assert_eq!(asked_for, Some(env.get_url()));
    }

    #[rstest]
    fn a_store_without_a_session_gives_no_credential(sessions: TempDir) {
        assert!(matches!(
            CredentialSource::StoredLogin
                .resolve_with(&Env::Production, |_| Ok(store_in(&sessions))),
            Err(ConsoleError::NoCredentials)
        ));
    }

    #[rstest]
    fn a_stored_session_is_the_credential(sessions: TempDir) {
        store_in(&sessions)
            .save(&StoredAppSession {
                access_token: AccessToken::new("tcl_at_stored"),
                access_token_expires_at: SystemTime::UNIX_EPOCH,
                refresh_token: RefreshToken::new("tcl_rt_stored"),
                refresh_token_expires_at: SystemTime::UNIX_EPOCH,
            })
            .unwrap();

        assert!(matches!(
            CredentialSource::StoredLogin
                .resolve_with(&Env::Production, |_| Ok(store_in(&sessions))),
            Ok(TracelCredentials::AppSession(_))
        ));
    }

    #[test]
    fn an_explicit_credential_needs_no_session_store() {
        let source = CredentialSource::Explicit(TracelCredentials::api_key(API_KEY));

        assert!(matches!(
            source.resolve_with(&Env::Production, no_store),
            Ok(TracelCredentials::ApiKey(key)) if key == API_KEY
        ));
    }

    #[test]
    fn tracel_env_unset_or_empty_names_production() {
        assert!(matches!(parse_env(None), Ok(Env::Production)));
        assert!(matches!(parse_env(Some("")), Ok(Env::Production)));
    }

    #[test]
    fn tracel_env_accepts_the_spellings_of_the_tracel_cli() {
        for value in ["Production", "production"] {
            assert!(
                matches!(parse_env(Some(value)), Ok(Env::Production)),
                "{value}"
            );
        }
        for value in ["Development", "development"] {
            assert!(
                matches!(parse_env(Some(value)), Ok(Env::Development)),
                "{value}"
            );
        }
        for value in ["Staging(2)", "staging-2"] {
            assert!(
                matches!(parse_env(Some(value)), Ok(Env::Staging(2))),
                "{value}"
            );
        }
    }

    #[test]
    fn an_unrecognized_tracel_env_is_an_error_naming_the_variable_and_value() {
        for value in [
            "DEVELOPMENT",
            "dev",
            " production",
            "Staging(x)",
            "Staging()",
            "Staging(1",
            "Staging(256)",
            "Staging(+2)",
            "staging-",
            "staging--1",
            "staging-+2",
            "staging-256",
        ] {
            let error = parse_env(Some(value)).unwrap_err();

            assert!(
                matches!(
                    &error,
                    ConsoleError::InvalidSetting { variable: "TRACEL_ENV", value: held, .. }
                        if held == value
                ),
                "{value}: {error:?}"
            );
            assert!(error.to_string().contains("TRACEL_ENV"), "{error}");
            assert!(error.to_string().contains(value), "{error}");
        }
    }
}
