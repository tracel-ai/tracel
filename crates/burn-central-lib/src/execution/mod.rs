//! Execution module for the Burn Central library
//!
//! This module provides two distinct capabilities:
//! 1. **Local Execution**: Core functionality to build and run functions locally
//! 2. **Job Submission**: Submit jobs to the Burn Central platform for remote execution
//!
//! These are separate concerns - remote jobs are eventually executed locally by compute providers
//! using the same local execution core.

pub mod local;
pub mod submission;

use crate::generation::backend::BackendType;
use crate::tools::function_discovery::FunctionMetadata;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Types of procedures that can be executed
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ProcedureType {
    Training,
    Inference,
}

impl std::fmt::Display for ProcedureType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProcedureType::Training => write!(f, "training"),
            ProcedureType::Inference => write!(f, "inference"),
        }
    }
}

/// Build profiles supported
#[derive(Debug, Clone, PartialEq)]
pub enum BuildProfile {
    Debug,
    Release,
}

impl BuildProfile {
    pub fn as_cargo_arg(&self) -> &'static str {
        match self {
            BuildProfile::Debug => "--profile=dev",
            BuildProfile::Release => "--profile=release",
        }
    }
}

impl Default for BuildProfile {
    fn default() -> Self {
        BuildProfile::Release
    }
}

/// Configuration for submitting a job to the platform
#[derive(Debug, Clone)]
pub struct JobSubmissionConfig {
    /// The function to execute
    pub function: String,
    /// Backend to use for execution
    pub backend: Option<BackendType>,
    /// Configuration file path
    pub config_file: Option<String>,
    /// Key-value overrides for configuration
    pub overrides: HashMap<String, serde_json::Value>,
    /// Type of procedure to execute
    pub procedure_type: ProcedureType,
    /// Code version to use for execution
    pub code_version: String,
    /// Compute provider to execute on
    pub compute_provider: String,
    /// Project namespace
    pub namespace: String,
    /// Project name
    pub project: String,
    /// API key for authentication
    pub api_key: String,
    /// API endpoint
    pub api_endpoint: String,
}

impl JobSubmissionConfig {
    /// Create a new job submission config
    pub fn new(
        function: String,
        procedure_type: ProcedureType,
        code_version: String,
        compute_provider: String,
        namespace: String,
        project: String,
        api_key: String,
        api_endpoint: String,
    ) -> Self {
        Self {
            function,
            procedure_type,
            code_version,
            compute_provider,
            namespace,
            project,
            api_key,
            api_endpoint,
            backend: None,
            config_file: None,
            overrides: HashMap::new(),
        }
    }

    /// Set the backend
    pub fn with_backend(mut self, backend: BackendType) -> Self {
        self.backend = Some(backend);
        self
    }

    /// Set the config file
    pub fn with_config_file<S: Into<String>>(mut self, config_file: S) -> Self {
        self.config_file = Some(config_file.into());
        self
    }

    /// Add configuration overrides
    pub fn with_overrides(mut self, overrides: HashMap<String, serde_json::Value>) -> Self {
        self.overrides = overrides;
        self
    }
}

/// Result of a job submission
#[derive(Debug)]
pub struct JobSubmissionResult {
    /// Whether the submission was successful
    pub success: bool,
    /// Job ID or confirmation message
    pub output: Option<String>,
    /// Error message if submission failed
    pub error: Option<String>,
}

impl JobSubmissionResult {
    /// Create a successful submission result
    pub fn success(output: Option<String>) -> Self {
        Self {
            success: true,
            output,
            error: None,
        }
    }

    /// Create a failed submission result
    pub fn failure(error: String) -> Self {
        Self {
            success: false,
            output: None,
            error: Some(error),
        }
    }
}

/// Error types specific to execution
#[derive(thiserror::Error, Debug)]
pub enum ExecutionError {
    #[error("Build failed: {0}")]
    BuildFailed(String),

    #[error("Runtime execution failed: {0}")]
    RuntimeFailed(String),

    #[error("Function not found: {0}")]
    FunctionNotFound(String),

    #[error("Invalid configuration: {0}")]
    InvalidConfiguration(String),

    #[error("Job submission failed: {0}")]
    JobSubmissionFailed(String),

    #[error("Project not initialized")]
    ProjectNotInitialized,

    #[error("Authentication failed")]
    AuthenticationFailed,
}

/// Parse a key=value string into a key-value pair
pub fn parse_key_value(s: &str) -> crate::Result<(String, serde_json::Value)> {
    let (key, value) = s
        .split_once('=')
        .ok_or_else(|| anyhow::anyhow!("Invalid key=value format: {}", s))?;

    let json_value = serde_json::from_str(value)
        .unwrap_or_else(|_| serde_json::Value::String(value.to_string()));

    Ok((key.to_string(), json_value))
}

/// Validate that a function exists in the available functions
pub fn validate_function(
    function: &str,
    available_functions: &[FunctionMetadata],
) -> crate::Result<()> {
    let function_names: Vec<&str> = available_functions
        .iter()
        .map(|f| f.fn_name.as_str())
        .collect();

    if !function_names.contains(&function) {
        return Err(ExecutionError::FunctionNotFound(format!(
            "Function '{}' not found. Available functions: {:?}",
            function, function_names
        ))
        .into());
    }

    Ok(())
}

/// Get training functions from a list of function metadata
pub fn get_training_functions(functions: &[FunctionMetadata]) -> Vec<String> {
    functions
        .iter()
        .filter(|f| f.proc_type.to_lowercase() == "training")
        .map(|f| f.routine_name.clone())
        .collect()
}

/// Get inference functions from a list of function metadata
pub fn get_inference_functions(functions: &[FunctionMetadata]) -> Vec<String> {
    functions
        .iter()
        .filter(|f| f.proc_type.to_lowercase() == "inference")
        .map(|f| f.routine_name.clone())
        .collect()
}
