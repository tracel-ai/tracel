use crate::app_config::{AppConfig, Credentials};
use crate::burn_dir::BurnDir;
use crate::burn_dir::project::BurnCentralProject;
use crate::discovery::functions::FunctionRegistry;
use crate::terminal::Terminal;
use crate::{cargo, config::Config};
use anyhow::Context;
use burn_central_client::BurnCentral;
use burn_central_client::credentials::BurnCentralCredentials;
use burn_central_client::schemas::ProjectPath;
use std::path::{Path, PathBuf};

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

    pub fn create_client(&self) -> Result<BurnCentral, ClientCreationError> {
        let api_key = self
            .get_api_key()
            .ok_or(ClientCreationError::NoCredentials)?;

        let creds = BurnCentralCredentials::new(api_key.to_owned());
        let builder = BurnCentral::builder(creds).with_endpoint(self.api_endpoint.clone());

        builder.build().map_err(|e| match e {
            burn_central_client::InitError::Client(e) if e.is_login_error() => {
                ClientCreationError::InvalidCredentials
            }
            burn_central_client::InitError::Client(e) if e.code().is_some() => {
                ClientCreationError::InvalidCredentials
            }
            _ => ClientCreationError::ServerConnectionError(e.to_string()),
        })
    }

    pub fn package_name(&self) -> &str {
        self.project_metadata.user_crate_name.as_str()
    }

    pub fn generated_crate_name(&self) -> &str {
        &self.project_metadata.generated_crate_name
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

    pub fn terminal_mut(&mut self) -> &mut Terminal {
        &mut self.terminal
    }
}

pub struct ProjectContext {
    pub user_crate_name: String,
    pub user_crate_dir: PathBuf,
    pub generated_crate_name: String,
    pub build_profile: String,
    pub burn_dir: BurnDir,
    pub project: Option<BurnCentralProject>,
}

impl ProjectContext {
    pub fn load_from_manifest(manifest_path: &Path) -> Self {
        // assert that the manifest path is a file
        assert!(manifest_path.is_file());
        assert!(manifest_path.ends_with("Cargo.toml"));
        // get the project name from the Cargo.toml
        let toml_str = std::fs::read_to_string(manifest_path).expect("Cargo.toml should exist");
        let manifest_document =
            toml::de::from_str::<toml::Value>(&toml_str).expect("Cargo.toml should be valid");

        let user_crate_name = manifest_document["package"]["name"]
            .as_str()
            .expect("Package name should exist")
            .to_string();
        let generated_crate_name = format!("{user_crate_name}_gen");

        let user_crate_dir = manifest_path
            .parent()
            .expect("Project directory should exist")
            .to_path_buf();
        let burn_dir = BurnDir::new(&user_crate_dir);
        burn_dir
            .init()
            .expect("Burn directory should be initialized");

        Self {
            user_crate_name,
            user_crate_dir,
            generated_crate_name,
            build_profile: "release".to_string(),
            burn_dir,
            project: None,
        }
    }

    pub fn load_project(&mut self) -> anyhow::Result<()> {
        self.project = Some(self.burn_dir.load_project()?);
        Ok(())
    }
}
