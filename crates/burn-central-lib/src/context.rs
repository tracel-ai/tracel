use crate::config::Config;
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

/// Core context for Burn Central operations, without CLI-specific dependencies
pub struct BurnCentralContext {
    api_endpoint: url::Url,
    creds: Option<Credentials>,
}

impl BurnCentralContext {
    pub fn new(config: &Config) -> Self {
        Self {
            api_endpoint: config
                .api_endpoint
                .parse::<url::Url>()
                .expect("API endpoint should be valid"),
            creds: None,
        }
    }

    pub fn init(self) -> Self {
        // Credential loading is now handled by the CLI layer
        self
    }

    pub fn set_credentials(&mut self, creds: Credentials) {
        self.creds = Some(creds);
        // Credential persistence is now handled by the CLI layer
    }

    pub fn get_api_key(&self) -> Option<&str> {
        self.creds.as_ref().map(|creds| creds.api_key.as_str())
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

    pub fn has_credentials(&self) -> bool {
        self.creds.is_some()
    }

    pub fn get_credentials(&self) -> Option<&Credentials> {
        self.creds.as_ref()
    }
}
