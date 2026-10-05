use std::fmt;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use tracel_artifact::ReqwestTransferClient;
use tracel_client::{
    ClientError,
    console::{Client, TracelCredentials},
};
use tracel_datasets::Datasets;
use tracel_experiment::ExperimentModule;
use tracel_inference::InferenceModule;
use tracel_models::Models;
use url::Url;

use crate::datasets::ConsoleDatasetOps;
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
pub struct ConsoleInner {
    pub client: Client,
    pub transfer_client: ReqwestTransferClient,
    credential_ends_at: Option<SystemTime>,
}

/// A project location bound to a console connection.
pub struct ProjectScope {
    pub console: Arc<ConsoleInner>,
    pub owner: String,
    pub project: String,
}

impl Console {
    /// Connects to the console and verifies the credentials.
    ///
    /// Warns when the credential stops being accepted within a day, since a run that
    /// outlives it stops with [`ConsoleError::SessionExpired`].
    pub fn connect(credentials: &TracelCredentials) -> Result<Self, ConsoleError> {
        let client = Client::connect(crate::env::from_environment(), credentials)?;
        let credential_ends_at = credential_end(credentials, &client);
        warn_when_ending_soon(credentials, credential_ends_at);

        Ok(Self {
            inner: Arc::new(ConsoleInner {
                client,
                transfer_client: ReqwestTransferClient::new(),
                credential_ends_at,
            }),
        })
    }

    /// When the credential this console connected with stops being accepted: an API
    /// key's expiry, or the end of the app session the `tracel` CLI signed in. `None`
    /// for a key without expiry and for an access token whose renewal belongs to the
    /// caller.
    pub fn credential_ends_at(&self) -> Option<SystemTime> {
        self.inner.credential_ends_at
    }

    /// Returns the normalized console API base URL.
    pub fn base_url(&self) -> &Url {
        self.inner.client.base_url()
    }

    /// Returns the current user, or `None` when the console no longer accepts the credential.
    pub fn me(&self) -> Result<Option<User>, ConsoleError> {
        match self.inner.client.get_current_user() {
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
            .client
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
        let projects = match namespace.kind {
            NamespaceKind::User => self.inner.client.list_user_projects(&namespace.name),
            NamespaceKind::Organization => self
                .inner
                .client
                .list_organization_projects(&namespace.name),
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
                console: Arc::clone(&self.inner),
                owner: owner.into(),
                project: project.into(),
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
        self.scope
            .console
            .client
            .get_project(&self.scope.owner, &self.scope.project)
            .map_err(ConsoleError::from)
            .and_then(Project::try_from)
    }

    /// Returns dataset operations already scoped to this project without performing I/O.
    pub fn datasets(&self) -> Datasets {
        Datasets::new(Arc::new(ConsoleDatasetOps {
            scope: Arc::clone(&self.scope),
        }))
    }

    /// Returns model operations already scoped to this project without performing I/O.
    pub fn models(&self) -> Models {
        Models::new(Arc::new(ConsoleModelOps {
            scope: Arc::clone(&self.scope),
        }))
    }

    /// Builds an experiment provider scoped to this project.
    pub fn experiments(&self) -> ExperimentModule {
        ExperimentModule::new(Arc::new(ConsoleExperimentProvider::new(Arc::clone(
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
