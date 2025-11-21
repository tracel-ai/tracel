//! Burn Central Library
//!
//! This crate provides the core functionality for Burn Central project management,
//! code generation, and execution. It separates local execution (the core that builds
//! and runs functions) from job submission (submitting jobs to the platform).

pub mod compute_provider;
pub mod config;
pub mod context;
pub mod entity;
pub mod execution;
pub mod generation;
pub mod logging;
pub mod tools;

// Re-export commonly used types for convenience
pub use config::Config;
pub use context::{BurnCentralContext, ClientCreationError, Credentials};
pub use entity::projects::ProjectContext;
pub use execution::{
    BuildProfile, JobSubmissionConfig, JobSubmissionResult, ProcedureType,
    local::{LocalExecutionConfig, LocalExecutionResult, LocalExecutor},
    submission::{JobInfo, JobStatus, JobSubmissionBuilder, JobSubmissionClient},
};
pub use generation::GeneratedCrate;
pub use tools::function_discovery::{FunctionDiscovery, FunctionMetadata};
pub use tools::functions_registry::FunctionRegistry;

// Re-export client types
pub use burn_central_client::{Client, request, response};

/// Result type commonly used throughout the library
pub type Result<T> = anyhow::Result<T>;

/// Core local execution functionality - used by both CLI and compute providers
pub mod local_execution {
    pub use crate::execution::local::{LocalExecutionConfig, LocalExecutionResult, LocalExecutor};
    pub use crate::execution::{BuildProfile, ProcedureType};
}

/// Job submission functionality - for submitting jobs to the platform
pub mod job_submission {
    pub use crate::execution::submission::{
        JobInfo, JobStatus, JobSubmissionBuilder, JobSubmissionClient,
    };
    pub use crate::execution::{JobSubmissionConfig, JobSubmissionResult};
}

/// Core functionality for project management and operations
pub mod core {
    pub use crate::context::BurnCentralContext;
    pub use crate::entity::projects::ProjectContext;
    pub use crate::execution::local::LocalExecutor;
    pub use crate::execution::submission::JobSubmissionClient;
    pub use crate::generation::GeneratedCrate;
    pub use crate::tools::function_discovery::FunctionDiscovery;
    pub use crate::tools::functions_registry::FunctionRegistry;
}

/// Utilities for working with Cargo projects and Git repositories
pub mod utils {
    pub use crate::tools::cargo;
    pub use crate::tools::git;
}
