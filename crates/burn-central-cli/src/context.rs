use crate::app_config::{AppConfig, Environment};
use crate::tools::terminal::Terminal;
use burn_central_client::Client;
use burn_central_lib::{BurnCentralContext, ClientCreationError, Config, Credentials};

/// CLI-specific context that wraps the library context with terminal functionality
pub struct CliContext {
    terminal: Terminal,
    core_context: BurnCentralContext,
    environment: Environment,
}

impl CliContext {
    pub fn new(terminal: Terminal, config: &Config, environment: Environment) -> Self {
        Self {
            terminal,
            core_context: BurnCentralContext::new(config),
            environment,
        }
    }

    pub fn init(mut self) -> Self {
        // Load credentials from AppConfig and inject into core context
        if let Ok(app_config) = AppConfig::new(self.environment()) {
            if let Ok(Some(creds)) = app_config.load_credentials() {
                self.core_context.set_credentials(creds);
            }
        }
        self.core_context = self.core_context.init();
        self
    }

    pub fn set_credentials(&mut self, creds: Credentials) {
        // Save credentials to AppConfig
        if let Ok(app_config) = AppConfig::new(self.environment()) {
            let _ = app_config.save_credentials(&creds);
        }
        self.core_context.set_credentials(creds);
    }

    pub fn get_api_key(&self) -> Option<&str> {
        self.core_context.get_api_key()
    }

    pub fn create_client(&self) -> Result<Client, ClientCreationError> {
        self.core_context.create_client()
    }

    pub fn set_config(&mut self, config: &Config) {
        self.core_context.set_config(config);
    }

    pub fn get_api_endpoint(&self) -> &url::Url {
        self.core_context.get_api_endpoint()
    }

    pub fn get_frontend_endpoint(&self) -> url::Url {
        self.core_context.get_frontend_endpoint()
    }

    pub fn terminal(&self) -> &Terminal {
        &self.terminal
    }

    pub fn environment(&self) -> Environment {
        self.environment
    }

    pub fn has_credentials(&self) -> bool {
        self.core_context.has_credentials()
    }

    pub fn get_credentials(&self) -> Option<&Credentials> {
        self.core_context.get_credentials()
    }

    /// Access the underlying library context for advanced operations
    pub fn core_context(&self) -> &BurnCentralContext {
        &self.core_context
    }

    /// Mutable access to the underlying library context for advanced operations
    pub fn core_context_mut(&mut self) -> &mut BurnCentralContext {
        &mut self.core_context
    }

    pub fn get_burn_dir_name(&self) -> &str {
        match self.environment {
            Environment::Development => ".burn-dev",
            Environment::Production => ".burn",
        }
    }
}
