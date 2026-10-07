use std::fmt;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, SystemTime};

use tracel_artifact::ReqwestTransferClient;
use tracel_client::{
    ClientError,
    console::{Client, Env, TracelCredentials, user::response::UserResponseSchema},
};
use tracel_datasets::DatasetRegistry;
use tracel_experiment::Experiments;
use tracel_inference::InferenceModule;
use tracel_models::ModelRegistry;
use url::Url;

use crate::datasets::ConsoleDatasetOps;
use crate::env::{CredentialSource, ProjectRef, env_from_environment};
use crate::experiment::ConsoleExperimentProvider;
use crate::inference::ConsoleInferenceProvider;
use crate::models::ConsoleModelOps;
use crate::{ConsoleError, Namespace, NamespaceKind, Organization, Project, User};

/// A blocking client rooted at one Tracel console URL.
#[derive(Clone)]
pub struct Console {
    inner: Arc<ConsoleInner>,
}

/// How long before a credential ends a connection warns about it.
const CREDENTIAL_END_WARNING: Duration = Duration::from_secs(24 * 3600);

/// Resources shared by every handle derived from a console connection.
struct ConsoleInner {
    env: Env,
    base_url: Url,
    credentials: TracelCredentials,
    transfer_client: ReqwestTransferClient,
    connected: OnceLock<Connected>,
    connecting: Mutex<()>,
}

/// A client whose credential the console accepted.
struct Connected {
    client: Client,
    credential_ends_at: Option<SystemTime>,
}

impl ConsoleInner {
    /// Returns the client, verifying the credential with the console on the first call.
    ///
    /// A failed verification is not kept, so the next call tries again.
    fn client(&self) -> Result<&Client, ClientError> {
        self.connected().map(|connected| &connected.client)
    }

    fn connected(&self) -> Result<&Connected, ClientError> {
        if let Some(connected) = self.connected.get() {
            return Ok(connected);
        }

        // Concurrent first requests share one verification.
        let _connecting = self
            .connecting
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(connected) = self.connected.get() {
            return Ok(connected);
        }

        let client = Client::connect(self.env.clone(), &self.credentials)?;
        let credential_ends_at = credential_end(&self.credentials, &client);
        warn_when_ending_soon(&self.credentials, credential_ends_at);

        Ok(self.connected.get_or_init(|| Connected {
            client,
            credential_ends_at,
        }))
    }

    /// Returns the current user. Verifying the credential reads it, so a call that verifies does
    /// not ask again.
    fn current_user(&self) -> Result<UserResponseSchema, ClientError> {
        match self.connected.get() {
            Some(connected) => connected.client.get_current_user(),
            None => self.client().map(|client| client.user().clone()),
        }
    }
}

/// A project location bound to a console connection.
pub struct ProjectScope {
    console: Console,
    pub owner: String,
    pub project: String,
    /// The kind of namespace that owns the project, once read from the console.
    owner_kind: OnceLock<NamespaceKind>,
}

impl ProjectScope {
    /// Returns the console client, verifying the credential on the first request.
    pub fn client(&self) -> Result<&Client, ClientError> {
        self.console.inner.client()
    }

    /// Returns the console this project is reached through.
    pub fn console(&self) -> &Console {
        &self.console
    }

    /// Fetches the project's details.
    pub fn fetch(&self) -> Result<Project, ConsoleError> {
        let project = self.client()?.get_project(&self.owner, &self.project)?;
        let project = Project::try_from(project)?;
        let _ = self.owner_kind.set(project.namespace.kind);
        Ok(project)
    }

    /// Returns the kind of namespace that owns the project, fetching the project on the first
    /// call.
    pub fn owner_kind(&self) -> Result<NamespaceKind, ConsoleError> {
        match self.owner_kind.get() {
            Some(kind) => Ok(*kind),
            None => self.fetch().map(|project| project.namespace.kind),
        }
    }

    /// Returns the client that moves files to and from presigned URLs.
    pub fn transfer_client(&self) -> &ReqwestTransferClient {
        &self.console.inner.transfer_client
    }
}

impl Console {
    /// Binds to the console `env` names, with `credentials`, without performing I/O.
    ///
    /// The first request verifies `credentials`, and fails with [`ConsoleError::SessionExpired`]
    /// when the console refuses them. It warns when the credential stops being accepted within a
    /// day, since a run that outlives it stops with [`ConsoleError::SessionExpired`].
    pub fn connect(env: Env, credentials: TracelCredentials) -> Self {
        Self {
            inner: Arc::new(ConsoleInner {
                base_url: env.get_url(),
                env,
                credentials,
                transfer_client: ReqwestTransferClient::new(),
                connected: OnceLock::new(),
                connecting: Mutex::new(()),
            }),
        }
    }

    /// Binds to the console `TRACEL_ENV` names, with the credential the environment provides,
    /// without performing network I/O.
    ///
    /// `TRACEL_ENV` unset or empty names the production console, as [`env_from_environment`]
    /// reads it. The credential is `TRACEL_API_KEY`, or else the session `tracel login` stored for
    /// that console, as [`CredentialSource::from_env`] chooses. Fails with
    /// [`ConsoleError::InvalidSetting`] when `TRACEL_ENV` names no console, and with
    /// [`ConsoleError::NoCredentials`] when there is no credential. The first request verifies
    /// the credential, as with [`connect`](Self::connect).
    pub fn from_env() -> Result<Self, ConsoleError> {
        let env = env_from_environment()?;
        let credentials = CredentialSource::from_env().resolve(&env)?;
        Ok(Self::connect(env, credentials))
    }

    /// When the credential this console uses stops being accepted: an API key's expiry, or the
    /// end of the app session the `tracel` CLI signed in. `None` for a key without expiry and for
    /// an access token whose renewal belongs to the caller.
    ///
    /// Verifies the credential first when no request has yet.
    pub fn credential_ends_at(&self) -> Result<Option<SystemTime>, ConsoleError> {
        Ok(self.inner.connected()?.credential_ends_at)
    }

    /// Returns the normalized console API base URL.
    pub fn base_url(&self) -> &Url {
        &self.inner.base_url
    }

    /// Returns the current user, or `None` when the console no longer accepts the credential.
    pub fn me(&self) -> Result<Option<User>, ConsoleError> {
        match self.inner.current_user() {
            Ok(user) => Ok(Some(User {
                id: user._id,
                username: user.username,
                email: user.email,
                namespace: Namespace::user(user.namespace),
            })),
            Err(ClientError::Unauthenticated) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// Lists organizations available to the current session.
    pub fn organizations(&self) -> Result<Vec<Organization>, ConsoleError> {
        self.inner
            .client()?
            .get_user_organizations()
            .map(|response| {
                response
                    .organizations
                    .into_iter()
                    .map(|organization| Organization {
                        name: organization.name,
                        namespace: Namespace::organization(organization.namespace),
                    })
                    .collect()
            })
            .map_err(Into::into)
    }

    /// Lists visible projects owned by a user or organization namespace.
    pub fn projects_of(
        &self,
        namespace: impl AsRef<Namespace>,
    ) -> Result<Vec<Project>, ConsoleError> {
        let namespace = namespace.as_ref();
        let client = self.inner.client()?;
        let projects = match namespace.kind {
            NamespaceKind::User => client.list_user_projects(&namespace.name),
            NamespaceKind::Organization => client.list_organization_projects(&namespace.name),
        }?;

        projects.into_iter().map(Project::try_from).collect()
    }

    /// Creates a project handle without performing I/O.
    pub fn project<O, P>(&self, owner: O, project: P) -> ProjectHandle
    where
        O: Into<String>,
        P: Into<String>,
    {
        ProjectHandle {
            scope: Arc::new(ProjectScope {
                console: self.clone(),
                owner: owner.into(),
                project: project.into(),
                owner_kind: OnceLock::new(),
            }),
        }
    }
}

fn credential_end(credentials: &TracelCredentials, client: &Client) -> Option<SystemTime> {
    match credentials {
        TracelCredentials::ApiKey(_) => client
            .user()
            .credential
            .expires_at
            .as_deref()
            .and_then(|expires_at| chrono::DateTime::parse_from_rfc3339(expires_at).ok())
            .map(SystemTime::from),
        TracelCredentials::AppSession(app_session) => app_session
            .stored()
            .ok()
            .flatten()
            .map(|stored| stored.refresh_token_expires_at),
        TracelCredentials::AccessToken(_) => None,
    }
}

fn warn_when_ending_soon(credentials: &TracelCredentials, ends_at: Option<SystemTime>) {
    let Some(left) = ends_at.and_then(|ends_at| ends_at.duration_since(SystemTime::now()).ok())
    else {
        return;
    };
    if left > CREDENTIAL_END_WARNING {
        return;
    }
    let hours = left.as_secs() / 3600;
    match credentials {
        TracelCredentials::ApiKey(_) => tracing::warn!(
            "The API key expires in {hours} h; a run that outlives it will stop. Create a new key for long runs."
        ),
        _ => tracing::warn!(
            "Your `tracel login` sign-in ends in {hours} h; a run that outlives it will stop. Run `tracel login` again, or set TRACEL_API_KEY for long runs."
        ),
    }
}

impl fmt::Debug for Console {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Console")
            .field("base_url", &self.base_url())
            .finish_non_exhaustive()
    }
}

/// A cheap view of one project that shares its console client's session.
#[derive(Clone)]
pub struct ProjectHandle {
    scope: Arc<ProjectScope>,
}

impl ProjectHandle {
    /// Returns the project [`ProjectRef::from_env`] names, on the console [`Console::from_env`]
    /// binds to, without performing network I/O.
    ///
    /// The owner namespace is `TRACEL_NAMESPACE` and the project name `TRACEL_PROJECT`; either
    /// one unset is read from `namespace` or `project` in `tracel.toml` in the current
    /// directory. Fails with [`ConsoleError::NoNamespace`] or [`ConsoleError::NoProject`] when
    /// neither names it.
    pub fn from_env() -> Result<Self, ConsoleError> {
        let console = Console::from_env()?;
        let project = ProjectRef::from_env()?;
        Ok(console.project(project.namespace, project.name))
    }

    /// Returns the console this project is reached through.
    pub fn console(&self) -> &Console {
        self.scope.console()
    }

    /// Returns the project's owner namespace.
    pub fn owner(&self) -> &str {
        &self.scope.owner
    }

    /// Returns the project name.
    pub fn name(&self) -> &str {
        &self.scope.project
    }

    /// Fetches project details.
    ///
    /// Private and nonexistent projects both return [`ConsoleError::NotFound`] because the
    /// console intentionally does not reveal which case applies.
    pub fn get(&self) -> Result<Project, ConsoleError> {
        self.scope.fetch()
    }

    /// Returns dataset operations already scoped to this project without performing I/O.
    pub fn datasets(&self) -> DatasetRegistry {
        DatasetRegistry::new(Arc::new(ConsoleDatasetOps {
            scope: Arc::clone(&self.scope),
        }))
    }

    /// Returns model operations already scoped to this project without performing I/O.
    pub fn models(&self) -> ModelRegistry {
        ModelRegistry::new(Arc::new(ConsoleModelOps {
            scope: Arc::clone(&self.scope),
        }))
    }

    /// Returns experiments scoped to this project without performing I/O.
    pub fn experiments(&self) -> Experiments {
        Experiments::new(Arc::new(ConsoleExperimentProvider::new(Arc::clone(
            &self.scope,
        ))))
    }

    /// Builds an inference module scoped to this project.
    ///
    /// Unlike [`datasets`](Self::datasets)/[`models`](Self::models), the returned module owns a
    /// background worker per inference group: build it once and reuse it, rather than calling
    /// this again for every request.
    pub fn inference(&self) -> InferenceModule {
        InferenceModule::new(Arc::new(ConsoleInferenceProvider::new(Arc::clone(
            &self.scope,
        ))))
    }
}

impl fmt::Debug for ProjectHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProjectHandle")
            .field("owner", &self.scope.owner)
            .field("project", &self.scope.project)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn handles_can_be_shared_across_threads() {
        assert_send_sync::<Console>();
        assert_send_sync::<ProjectHandle>();
    }

    #[test]
    fn binding_and_scoping_send_no_request() {
        let console = Console::connect(
            Env::Development,
            TracelCredentials::api_key("tcl_key_not_checked_until_the_first_request"),
        );
        let project = console.project("owner", "project");
        let _ = (
            project.datasets(),
            project.models(),
            project.experiments(),
            project.inference(),
        );

        assert!(console.inner.connected.get().is_none());
        assert!(Arc::ptr_eq(&project.console().inner, &console.inner));
        assert_eq!(console.base_url(), &Env::Development.get_url());
    }
}
