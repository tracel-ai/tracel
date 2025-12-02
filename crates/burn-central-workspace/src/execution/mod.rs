pub mod cancellable;
pub mod local;

use crate::tools::function_discovery::FunctionMetadata;
use serde::{Deserialize, Serialize};
use strum::EnumString;

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
#[derive(Default, Debug, Clone, PartialEq)]
pub enum BuildProfile {
    Debug,
    #[default]
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

#[derive(Debug, Clone, EnumString, Default, Deserialize, Serialize, PartialEq, Eq)]
#[strum(serialize_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum BackendType {
    #[default]
    Wgpu,
    Tch,
    Ndarray,
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

    #[error("Execution cancelled")]
    Cancelled,
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
