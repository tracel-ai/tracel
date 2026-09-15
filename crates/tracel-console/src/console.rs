use std::fmt;
use std::future::Future;
use std::sync::Arc;

use futures::Stream;
use tracel_artifact::HttpTransferClient;
use tracel_client::console::{Client, TracelCredentials};
use tracel_models::Models;
use tracel_task::{Job, MaybeSend, Runtime, Streaming};
use url::Url;

use crate::models::ConsoleModelOps;
use crate::{ConsoleError, Namespace, NamespaceKind, Organization, Project, User};

// Capabilities not yet handed back as jobs still bridge through blocking calls.
#[cfg(not(target_arch = "wasm32"))]
use {
    crate::datasets::ConsoleDatasetOps, crate::experiment::ConsoleExperimentProvider,
    crate::inference::ConsoleInferenceProvider, tracel_artifact::ReqwestTransferClient,
    tracel_datasets::Datasets, tracel_experiment::ExperimentModule,
    tracel_inference::InferenceModule,
};

/// A client rooted at one Tracel console URL.
///
/// Every operation is a [`Job`] the caller awaits, blocks on, polls, or spawns where they
/// choose. How the transport runs is the connection's [`Runtime`]: natively the tokio runtime
/// the caller connected from, or one of the connection's own, with each call attached to it so
/// that any executor can drive the result; in the browser, the event loop.
#[derive(Clone)]
pub struct Console {
    inner: Arc<ConsoleInner>,
}

/// Resources shared by every handle derived from a console connection.
pub struct ConsoleInner {
    pub client: Client,
    pub transfer: HttpTransferClient,
    /// Drives the transport's IO and runs the connection's actors.
    pub runtime: Arc<Runtime>,
    /// The blocking facade the ports whose contract is still synchronous go through.
    #[cfg(not(target_arch = "wasm32"))]
    pub transfer_client: ReqwestTransferClient,
}

impl ConsoleInner {
    async fn connect(credentials: TracelCredentials) -> Result<Self, ConsoleError> {
        let runtime = Arc::new(Runtime::acquire().expect("failed to start the console runtime"));
        let env = crate::env::from_environment();
        let client = runtime.attach(Client::connect(env, &credentials)).await?;

        Ok(Self {
            client,
            transfer: HttpTransferClient::new(),
            #[cfg(not(target_arch = "wasm32"))]
            transfer_client: ReqwestTransferClient::with_runtime(Arc::clone(&runtime)),
            runtime,
        })
    }

    /// Hands `call` back as a job any executor can drive, its IO driven by the runtime.
    pub fn attach<T, E, F>(&self, call: F) -> Job<T, E>
    where
        F: Future<Output = Result<T, E>> + MaybeSend + 'static,
    {
        Job::new(self.runtime.attach(call))
    }

    /// Hands `stream` back as items any executor can pull, its IO driven by the runtime.
    pub fn attach_stream<T, E, S>(&self, stream: S) -> Streaming<T, E>
    where
        S: Stream<Item = Result<T, E>> + MaybeSend + 'static,
    {
        Streaming::new(self.runtime.attach_stream(stream))
    }
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
    /// The runtime is acquired when the job is first driven: natively, the tokio runtime the
    /// caller is inside at that moment, or one of the connection's own.
    pub fn connect(credentials: &TracelCredentials) -> Job<Self, ConsoleError> {
        let credentials = credentials.clone();
        Job::new(async move {
            let inner = ConsoleInner::connect(credentials).await?;
            Ok(Self {
                inner: Arc::new(inner),
            })
        })
    }

    /// Logs out and consumes this console connection.
    ///
    /// This revokes the remote session used by this connection and its derived handles.
    pub fn logout(self) -> Job<(), ConsoleError> {
        self.call(|inner| async move { inner.client.clone().logout().await.map_err(Into::into) })
    }

    /// Returns the normalized console API base URL.
    pub fn base_url(&self) -> &Url {
        self.inner.client.base_url()
    }

    /// Returns the current user, or `None` when the session is absent or dead.
    ///
    /// A dead session is represented by the console as a successful `null` response and remains a
    /// value rather than [`ConsoleError::SessionExpired`].
    pub fn me(&self) -> Job<Option<User>, ConsoleError> {
        self.call(|inner| async move {
            inner
                .client
                .get_current_user()
                .await
                .map(|user| {
                    user.map(|user| User {
                        id: user._id,
                        username: user.username,
                        email: user.email,
                        namespace: Namespace::user(user.namespace),
                    })
                })
                .map_err(Into::into)
        })
    }

    /// Lists organizations available to the current session.
    pub fn organizations(&self) -> Job<Vec<Organization>, ConsoleError> {
        self.call(|inner| async move {
            inner
                .client
                .get_user_organizations()
                .await
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
        })
    }

    /// Lists visible projects owned by a user or organization namespace.
    pub fn projects_of(&self, namespace: impl AsRef<Namespace>) -> Job<Vec<Project>, ConsoleError> {
        let namespace = namespace.as_ref().clone();
        self.call(move |inner| async move {
            let client = &inner.client;
            let projects = match namespace.kind {
                NamespaceKind::User => client.list_user_projects(&namespace.name).await,
                NamespaceKind::Organization => {
                    client.list_organization_projects(&namespace.name).await
                }
            }?;

            projects.into_iter().map(Project::try_from).collect()
        })
    }

    /// Hands one client call back as a job.
    fn call<T, F, Fut>(&self, call: F) -> Job<T, ConsoleError>
    where
        F: FnOnce(Arc<ConsoleInner>) -> Fut,
        Fut: Future<Output = Result<T, ConsoleError>> + MaybeSend + 'static,
    {
        self.inner.attach(call(Arc::clone(&self.inner)))
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
    pub fn get(&self) -> Job<Project, ConsoleError> {
        let scope = Arc::clone(&self.scope);
        self.scope.console.attach(async move {
            scope
                .console
                .client
                .get_project(&scope.owner, &scope.project)
                .await
                .map_err(ConsoleError::from)
                .and_then(Project::try_from)
        })
    }

    /// Returns dataset operations already scoped to this project without performing I/O.
    #[cfg(not(target_arch = "wasm32"))]
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
    #[cfg(not(target_arch = "wasm32"))]
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
    #[cfg(not(target_arch = "wasm32"))]
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
