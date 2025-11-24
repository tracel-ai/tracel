use crate::app_config::{AppConfig, Environment};
use crate::config::Config;
use crate::tools::terminal::Terminal;
use burn_central_client::{BurnCentralCredentials, Client};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Credentials {
    pub api_key: String,
}

#[derive(thiserror::Error, Debug)]
pub enum ClientCreationError {
    #[error("No credentials found")]
    NoCredentials,
    #[error("Invalid credentials")]
    InvalidCredentials,
    #[error("Server connection error")]
    ServerConnectionError(String),
}

/// CLI-specific context that wraps the library context with terminal functionality
pub struct CliContext {
    terminal: Terminal,
    environment: Environment,
    api_endpoint: url::Url,
    creds: Option<Credentials>,
}

impl CliContext {
    pub fn new(terminal: Terminal, config: &Config, environment: Environment) -> Self {
        Self {
            terminal,
            environment,
            api_endpoint: config
                .api_endpoint
                .parse::<url::Url>()
                .expect("API endpoint should be valid"),
            creds: None,
        }
    }

    pub fn init(mut self) -> Self {
        // Load credentials from AppConfig and inject into core context
        if let Ok(app_config) = AppConfig::new(self.environment()) {
            if let Ok(Some(creds)) = app_config.load_credentials() {
                self.creds = Some(creds);
            }
        }
        self
    }

    pub fn set_credentials(&mut self, creds: Credentials) {
        // Save credentials to AppConfig
        if let Ok(app_config) = AppConfig::new(self.environment()) {
            _ = app_config.save_credentials(&creds);
        }
        self.creds = Some(creds);
    }

    pub fn get_api_key(&self) -> Option<&str> {
        self.creds.as_ref().map(|c| c.api_key.as_str())
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

    pub fn terminal(&self) -> &Terminal {
        &self.terminal
    }

    pub fn environment(&self) -> Environment {
        self.environment
    }

    pub fn get_burn_dir_name(&self) -> &str {
        match self.environment {
            Environment::Development => ".burn-dev",
            Environment::Production => ".burn",
        }
    }
}
