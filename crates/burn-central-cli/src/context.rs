use crate::app_config::{AppConfig, Credentials, Environment};
use crate::config::Config;
use crate::tools::terminal::Terminal;
use burn_central_client::{BurnCentralCredentials, Client};

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
    environment: Environment,
}

impl CliContext {
    pub fn new(terminal: Terminal, config: &Config, environment: Environment) -> Self {
        Self {
            terminal,
            api_endpoint: config
                .api_endpoint
                .parse::<url::Url>()
                .expect("API endpoint should be valid"),
            creds: None,
            environment,
        }
    }

    pub fn init(mut self) -> Self {
        let entry_res = AppConfig::new(self.environment);
        if let Ok(entry) = entry_res {
            if let Ok(Some(api_key)) = entry.load_credentials() {
                self.creds = Some(api_key);
            }
        }
        self
    }

    pub fn set_credentials(&mut self, creds: Credentials) {
        self.creds = Some(creds);
        let app_config = AppConfig::new(self.environment).expect("AppConfig should be created");
        app_config
            .save_credentials(self.creds.as_ref().unwrap())
            .expect("Credentials should be saved");
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

    pub fn terminal(&self) -> &Terminal {
        &self.terminal
    }

    pub fn environment(&self) -> Environment {
        self.environment
    }
}
