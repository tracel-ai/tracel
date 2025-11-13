use crate::app_config::{AppConfig, Credentials};
use crate::config::Config;
use crate::entity::projects::ProjectContext;
use crate::entity::projects::burn_dir::BurnDir;
use crate::entity::projects::project_path::ProjectPath;
use crate::tools::cargo;
use crate::tools::functions_registry::FunctionRegistry;
use crate::tools::terminal::Terminal;
use anyhow::Context;
use burn_central_api::{BurnCentralCredentials, Client};
use std::path::PathBuf;

#[derive(thiserror::Error, Debug)]
pub enum ClientCreationError {
    #[error("No credentials found")]
    NoCredentials,
    #[error("Invalid credentials")]
    InvalidCredentials,
    #[error("Server connection error")]
    ServerConnectionError(String),
}

pub struct CliContext {
    terminal: Terminal,
    api_endpoint: url::Url,
    creds: Option<Credentials>,
    project_metadata: ProjectContext,
    pub function_registry: FunctionRegistry,
}

impl CliContext {
    pub fn new(
        terminal: Terminal,
        config: &Config,
        project_metadata: ProjectContext,
        function_registry: FunctionRegistry,
    ) -> Self {
        Self {
            terminal,
            api_endpoint: config
                .api_endpoint
                .parse::<url::Url>()
                .expect("API endpoint should be valid"),
            creds: None,
            project_metadata,
            function_registry,
        }
    }

    pub fn init(mut self) -> Self {
        let entry_res = AppConfig::new();
        if let Ok(entry) = entry_res {
            if let Ok(Some(api_key)) = entry.load_credentials() {
                self.creds = Some(api_key);
            }
        }
        self
    }

    pub fn set_credentials(&mut self, creds: Credentials) {
        self.creds = Some(creds);
        let app_config = AppConfig::new().expect("AppConfig should be created");
        app_config
            .save_credentials(self.creds.as_ref().unwrap())
            .expect("Credentials should be saved");
    }

    pub fn get_api_key(&self) -> Option<&str> {
        self.creds.as_ref().map(|creds| creds.api_key.as_str())
    }

    pub fn get_project_path(&self) -> anyhow::Result<ProjectPath> {
        let project_info = self
            .project_metadata
            .project
            .as_ref()
            .context("Could not load project metadata")?;
        Ok(ProjectPath::new(
            project_info.owner.clone(),
            project_info.name.clone(),
        ))
    }

    pub fn create_client(&self) -> Result<Client, ClientCreationError> {
        let api_key = self
            .get_api_key()
            .ok_or(ClientCreationError::NoCredentials)?;

        let creds = BurnCentralCredentials::new(api_key.to_owned());
        let client = Client::new(self.api_endpoint.clone(), &creds);

        client.map_err(|e| {
            if e.is_login_error() {
                ClientCreationError::InvalidCredentials
            } else {
                ClientCreationError::ServerConnectionError(e.to_string())
            }
        })
    }

    pub fn package_name(&self) -> &str {
        self.project_metadata.user_crate_name.as_str()
    }

    pub fn generated_crate_name(&self) -> &str {
        &self.project_metadata.generated_crate_name
    }

    pub fn set_config(&mut self, config: &Config) {
        self.api_endpoint = config
            .api_endpoint
            .parse::<url::Url>()
            .expect("API endpoint should be valid");
    }

    pub fn get_api_endpoint(&self) -> &url::Url {
        &self.api_endpoint
    }

    pub fn get_frontend_endpoint(&self) -> url::Url {
        let host = self
            .api_endpoint
            .host_str()
            .expect("API endpoint should have a host");

        let mut host_url =
            url::Url::parse("https://example.com").expect("Base URL should be valid");
        host_url.set_host(Some(host)).expect("Host should be valid");
        host_url
            .set_scheme(self.api_endpoint.scheme())
            .expect("Scheme should be valid");
        host_url
    }

    pub fn cargo_cmd(&self) -> std::process::Command {
        let mut cmd = cargo::command();
        cmd.current_dir(self.cwd());
        cmd
    }

    pub fn get_artifacts_dir_path(&self) -> PathBuf {
        self.project_metadata.burn_dir.artifacts_dir()
    }

    pub fn metadata(&self) -> &ProjectContext {
        &self.project_metadata
    }

    pub fn burn_dir(&self) -> &BurnDir {
        &self.project_metadata.burn_dir
    }

    pub fn load_project(&mut self) -> anyhow::Result<()> {
        self.project_metadata.load_project()
    }

    pub fn cwd(&self) -> PathBuf {
        self.project_metadata.user_crate_dir.clone()
    }

    pub fn get_workspace_root(&self) -> anyhow::Result<PathBuf> {
        let metadata = cargo_metadata::MetadataCommand::new()
            .no_deps()
            .current_dir(self.cwd())
            .exec();

        match metadata {
            Ok(meta) => Ok(meta.workspace_root.into()),
            Err(e) => Err(anyhow::anyhow!("Unexpected error: {}", e)),
        }
    }

    pub fn terminal(&self) -> &Terminal {
        &self.terminal
    }
}
