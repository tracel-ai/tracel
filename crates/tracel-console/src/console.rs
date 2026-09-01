use std::fmt;
use std::sync::Arc;

use tracel_artifact::ReqwestTransferClient;
use tracel_client::console::{Client, TracelCredentials};
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

/// Resources shared by every handle derived from a console connection.
pub struct ConsoleInner {
    pub client: Client,
    pub transfer_client: ReqwestTransferClient,
}

/// A project location bound to a console connection.
pub struct ProjectScope {
    pub console: Arc<ConsoleInner>,
    pub owner: String,
    pub project: String,
}

impl Console {
    /// Connects to the console and verifies the credentials.
    pub fn connect(credentials: &TracelCredentials) -> Result<Self, ConsoleError> {
        let client = Client::connect(crate::env::from_environment(), credentials)?;

        Ok(Self {
            inner: Arc::new(ConsoleInner {
                client,
                transfer_client: ReqwestTransferClient::new(),
            }),
        })
    }

    /// Returns the normalized console API base URL.
    pub fn base_url(&self) -> &Url {
        self.inner.client.base_url()
    }

    /// Returns the current user, or `None` when the session is absent or dead.
    ///
    /// A dead session is represented by the console as a successful `null` response and remains a
    /// value rather than [`ConsoleError::SessionExpired`].
    pub fn me(&self) -> Result<Option<User>, ConsoleError> {
        self.inner
            .client
            .get_current_user()
            .map(|user| {
                user.map(|user| User {
                    id: user._id,
                    username: user.username,
                    email: user.email,
                    namespace: Namespace::user(user.namespace),
                })
            })
            .map_err(Into::into)
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
