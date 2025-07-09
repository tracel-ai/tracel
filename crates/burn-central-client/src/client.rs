//! This module provides the [BurnCentral] struct, which is used to interact with the Burn Central service.

use crate::api::Client;
use crate::api::ClientError;
use crate::credentials::BurnCentralCredentials;
use crate::experiment::{ExperimentRun, ExperimentTrackerError};
use crate::schemas::{
    BurnCentralCodeMetadata, CrateVersionMetadata, ExperimentPath, PackagedCrateData, ProjectPath,
    ProjectSchema, User,
};
use reqwest::Url;
use serde::Serialize;
use std::path::PathBuf;

/// Errors that can occur during the initialization of the [BurnCentral] client.
#[derive(Debug, thiserror::Error)]
pub enum InitError {
    /// Represents an error related to the client.
    #[error("Client error: {0}")]
    Client(#[from] ClientError),
    /// Represents an error when the endpoint URL is invalid.
    #[error("Failed to parse endpoint URL: {0}")]
    InvalidEndpointUrl(String),
    /// Represents an error when an environment variable is not set.
    #[error("Environment variable not set: {0}")]
    EnvNotSet(String),
}

#[derive(Debug, thiserror::Error)]
pub enum BurnCentralError {
    // Input validation errors
    #[error("Invalid experiment path: {0}")]
    InvalidExperimentPath(String),
    #[error("Invalid project path: {0}")]
    InvalidProjectPath(String),
    #[error("Invalid experiment number: {0}")]
    InvalidExperimentNumber(String),

    /// Represents an error related to client operations.
    ///
    /// This error variant is used to encapsulate client-specific errors along with additional context
    /// and the underlying source error for more detailed debugging.
    ///
    /// # Fields
    /// - `context` (String): A description or additional information about the client error context.
    /// - `source` (ClientError): The underlying source of the client error, providing more details about the cause.
    #[error("Client error: {context}")]
    Client {
        context: String,
        source: ClientError,
    },
    /// Represents an error related to the experiment tracker.
    #[error("Experiment error: {0}")]
    ExperimentTracker(#[from] ExperimentTrackerError),

    /// Error that should be used when the user is not logged in but tries to perform an operation that requires authentication.
    #[error("The user is not authenticated.")]
    Unauthenticated,

    /// Error that should be used when the client performs operations that can fail due to IO issues.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// Error that should be used when the client encounters an error that is not specifically handled.
    #[error("Internal error: {0}")]
    Internal(String),
}

/// This builder struct is used to create a [BurnCentral] client.
pub struct BurnCentralBuilder {
    endpoint: Option<String>,
    credentials: BurnCentralCredentials,
}

impl BurnCentralBuilder {
    /// Creates a new [BurnCentralBuilder] with the given credentials.
    pub fn new(credentials: impl Into<BurnCentralCredentials>) -> Self {
        BurnCentralBuilder {
            endpoint: None,
            credentials: credentials.into(),
        }
    }

    /// Sets the endpoint for the [BurnCentral] client.
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = Some(endpoint.into());
        self
    }

    /// Builds the [BurnCentral] client.
    pub fn build(self) -> Result<BurnCentral, InitError> {
        let url = match self.endpoint {
            Some(s) => s.parse::<Url>().map_err(|e| InitError::InvalidEndpointUrl(e.to_string()))?,
            None => Url::parse("https://central.burn.dev/api/").expect("Default URL should be valid"),
        };
        let client = Client::new(
            url,
            &self.credentials,
        )?;
        Ok(BurnCentral::new(client))
    }
}

/// This struct provides the main interface to interact with Burn Central.
pub struct BurnCentral {
    client: Client,
}

impl BurnCentral {
    /// Creates a new [BurnCentral] instance with the given credentials.
    pub fn login(credentials: impl Into<BurnCentralCredentials>) -> Result<Self, InitError> {
        let credentials = credentials.into();
        BurnCentralBuilder::new(credentials).build()
    }

    /// Creates a new [BurnCentralBuilder] to configure the client.
    pub fn builder(credentials: impl Into<BurnCentralCredentials>) -> BurnCentralBuilder {
        BurnCentralBuilder::new(credentials)
    }

    /// Creates a new [BurnCentral] instance from environment variables.
    ///
    /// This function reads the `BURN_CENTRAL_ENDPOINT` and `BURN_CENTRAL_API_KEY` environment variables.
    /// If the `BURN_CENTRAL_ENDPOINT` is not set, it defaults to `https://central.burn.dev/api/`.
    pub fn from_env() -> Result<Self, InitError> {
        let endpoint = std::env::var("BURN_CENTRAL_ENDPOINT")
            .unwrap_or_else(|_| "https://central.burn.dev/api/".to_string())
            .parse::<Url>()
            .map_err(|_| InitError::InvalidEndpointUrl("BURN_CENTRAL_ENDPOINT".to_string()))?;
        let credentials = BurnCentralCredentials::from_env()
            .map_err(|_| InitError::EnvNotSet("BURN_CENTRAL_API_KEY".to_string()))?;

        BurnCentralBuilder::new(credentials)
            .with_endpoint(endpoint.as_str())
            .build()
    }

    /// Creates a new instance of [BurnCentral] with the given [Client].
    fn new(client: Client) -> Self {
        BurnCentral { client }
    }

    pub fn find_project(
        &self,
        owner_name: impl AsRef<str>,
        project_name: impl AsRef<str>,
    ) -> Result<Option<ProjectSchema>, BurnCentralError> {
        let project = self
            .client
            .get_project(owner_name.as_ref(), project_name.as_ref())
            .map(Some)
            .or_else(|e| {
                if matches!(e, ClientError::NotFound) {
                    Ok(None)
                } else {
                    Err(e)
                }
            })
            .map_err(|e| BurnCentralError::Client {
                context: format!(
                    "Failed to get project {}/{}",
                    owner_name.as_ref(),
                    project_name.as_ref()
                ),
                source: e,
            })?
            .map(|project_schema| ProjectSchema {
                project_name: project_schema.project_name,
                namespace_name: project_schema.namespace_name,
                namespace_type: project_schema.namespace_type,
                description: project_schema.description,
                created_by: project_schema.created_by,
                created_at: project_schema.created_at,
                visibility: project_schema.visibility,
            });
        Ok(project)
    }

    pub fn create_project(
        &self,
        namespace_name: impl AsRef<str>,
        project_name: impl AsRef<str>,
        description: Option<&str>,
    ) -> Result<ProjectPath, BurnCentralError> {
        let project = self
            .client
            .create_project(namespace_name.as_ref(), project_name.as_ref(), description)
            .map_err(|e| BurnCentralError::Client {
                context: format!(
                    "Failed to create project {}/{}",
                    namespace_name.as_ref(),
                    project_name.as_ref()
                ),
                source: e,
            })?;

        let new_project_path = ProjectPath::new(project.namespace_name, project.project_name);
        Ok(new_project_path)
    }

    /// Returns the current user information.
    pub fn me(&self) -> Result<User, BurnCentralError> {
        let user = self.client.get_current_user().map_err(|e| {
            if matches!(e, ClientError::Unauthorized) {
                BurnCentralError::Unauthenticated
            } else {
                BurnCentralError::Client {
                    context: "Failed to get current user".to_string(),
                    source: e,
                }
            }
        })?;

        Ok(User {
            username: user.username,
            email: user.email,
        })
    }

    /// Start a new experiment. This will create a new experiment on the Burn Central backend and start it.
    pub fn start_experiment(
        &self,
        namespace: &str,
        project_name: &str,
        config: &impl Serialize,
    ) -> Result<ExperimentRun, BurnCentralError> {
        let experiment = self
            .client
            .create_experiment(namespace, project_name)
            .map_err(|e| BurnCentralError::Client {
                context: format!("Failed to create experiment for {namespace}/{project_name}"),
                source: e,
            })?;
        let experiment_path = ExperimentPath::try_from(format!(
            "{}/{}/{}",
            namespace, project_name, experiment.experiment_num
        ))?;

        self.client
            .start_experiment(
                namespace,
                &experiment.project_name,
                experiment.experiment_num,
                config,
            )
            .map_err(|e| BurnCentralError::Client {
                context: format!("Failed to start experiment {namespace}/{project_name}"),
                source: e,
            })?;

        println!("Experiment num: {}", experiment.experiment_num);

        ExperimentRun::new(self.client.clone(), experiment_path)
            .map_err(BurnCentralError::ExperimentTracker)
    }

    /// Upload a new version of a project to Burn Central.
    pub fn upload_new_project_version(
        &self,
        namespace: &str,
        project_name: &str,
        target_package_name: &str,
        code_metadata: BurnCentralCodeMetadata,
        crates_data: Vec<PackagedCrateData>,
        last_commit: &str,
    ) -> Result<String, BurnCentralError> {
        let (data, metadata): (Vec<(String, PathBuf)>, Vec<CrateVersionMetadata>) = crates_data
            .into_iter()
            .map(|krate| {
                (
                    (krate.name, krate.path),
                    CrateVersionMetadata {
                        checksum: krate.checksum,
                        metadata: krate.metadata,
                    },
                )
            })
            .unzip();

        let urls = self
            .client
            .publish_project_version_urls(
                namespace,
                project_name,
                target_package_name,
                code_metadata,
                metadata,
                last_commit,
            )
            .map_err(|e| BurnCentralError::Client {
                context: format!(
                    "Failed to get upload URLs for project {namespace}/{project_name}"
                ),
                source: e,
            })?;

        for (crate_name, file_path) in data.into_iter() {
            let url = urls
                .urls
                .get(&crate_name)
                .ok_or(BurnCentralError::Internal(format!(
                    "No upload URL found for crate: {crate_name}"
                )))?;

            let data = std::fs::read(&file_path).map_err(|e| {
                std::io::Error::new(
                    e.kind(),
                    format!("Failed to read crate file {}: {}", file_path.display(), e),
                )
            })?;

            self.client
                .upload_bytes_to_url(url, data)
                .map_err(|e| BurnCentralError::Client {
                    context: format!("Failed to upload crate {crate_name} to URL {url}"),
                    source: e,
                })?;
        }

        Ok(urls.project_version)
    }

    /// Start a remote job on the Burn Central backend.
    pub fn start_remote_job(
        &self,
        namespace: &str,
        project_name: &str,
        runner_group_name: String,
        project_version: &str,
        command: String,
    ) -> Result<(), BurnCentralError> {
        self.client
            .start_remote_job(
                &runner_group_name,
                namespace,
                project_name,
                project_version,
                command,
            )
            .map_err(|e| BurnCentralError::Client {
                context: format!(
                    "Failed to start remote job for {namespace}/{project_name}/{runner_group_name}"
                ),
                source: e,
            })?;

        Ok(())
    }
}
