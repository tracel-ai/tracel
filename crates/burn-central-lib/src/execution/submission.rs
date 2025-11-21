//! Job submission module for Burn Central platform
//!
//! This module handles submitting jobs to the Burn Central platform for execution
//! by compute providers. It is separate from local execution - jobs submitted here
//! will eventually be executed locally by compute providers using the same local
//! execution core.

use crate::{
    context::BurnCentralContext,
    entity::projects::ProjectContext,
    execution::{JobSubmissionConfig, JobSubmissionResult, ProcedureType},
};
use std::collections::HashMap;

/// Client for submitting jobs to the Burn Central platform
pub struct JobSubmissionClient<'a> {
    context: &'a BurnCentralContext,
    project: &'a ProjectContext,
}

impl<'a> JobSubmissionClient<'a> {
    /// Create a new job submission client
    pub fn new(context: &'a BurnCentralContext, project: &'a ProjectContext) -> Self {
        Self { context, project }
    }

    /// Submit a job to the platform for execution
    pub fn submit_job(&self, config: JobSubmissionConfig) -> crate::Result<JobSubmissionResult> {
        // TODO: Implement actual job submission using the burn-central-client
        // For now, return a success placeholder to avoid compilation errors

        log::info!("Submitting job: {}", config.function);
        log::info!("Compute provider: {}", config.compute_provider);
        log::info!("Code version: {}", config.code_version);

        // Simulate job submission
        let job_id = format!("job-{}", uuid::Uuid::new_v4());

        Ok(JobSubmissionResult::success(Some(job_id)))
    }

    /// Get the status of a submitted job
    pub fn get_job_status(&self, job_id: &str) -> crate::Result<JobStatus> {
        // TODO: Implement job status checking
        log::info!("Checking status for job: {}", job_id);
        Ok(JobStatus::Submitted)
    }

    /// Cancel a submitted job
    pub fn cancel_job(&self, job_id: &str) -> crate::Result<bool> {
        // TODO: Implement job cancellation
        log::info!("Cancelling job: {}", job_id);
        Ok(true)
    }

    /// List jobs for the current project
    pub fn list_jobs(&self) -> crate::Result<Vec<JobInfo>> {
        // TODO: Implement job listing
        log::info!("Listing jobs for project");
        Ok(vec![])
    }
}

/// Status of a submitted job
#[derive(Debug, Clone, PartialEq)]
pub enum JobStatus {
    /// Job has been submitted but not yet picked up
    Submitted,
    /// Job is currently being executed
    Running,
    /// Job completed successfully
    Completed,
    /// Job failed during execution
    Failed,
    /// Job was cancelled
    Cancelled,
}

/// Information about a job
#[derive(Debug, Clone)]
pub struct JobInfo {
    /// Unique job identifier
    pub job_id: String,
    /// Function that was executed
    pub function: String,
    /// Type of procedure (training/inference)
    pub procedure_type: ProcedureType,
    /// Current status
    pub status: JobStatus,
    /// When the job was submitted
    pub submitted_at: String,
    /// Compute provider executing the job
    pub compute_provider: String,
    /// Job results (if completed)
    pub results: Option<JobResults>,
}

/// Results of a completed job
#[derive(Debug, Clone)]
pub struct JobResults {
    /// Whether the job was successful
    pub success: bool,
    /// Output from the job
    pub output: Option<String>,
    /// Error message (if failed)
    pub error: Option<String>,
    /// When the job completed
    pub completed_at: String,
}

/// Builder for job submission configuration
pub struct JobSubmissionBuilder {
    function: String,
    procedure_type: ProcedureType,
    code_version: String,
    compute_provider: String,
    namespace: String,
    project: String,
    api_key: String,
    api_endpoint: String,
    backend: Option<crate::generation::backend::BackendType>,
    config_file: Option<String>,
    overrides: HashMap<String, serde_json::Value>,
}

impl JobSubmissionBuilder {
    /// Create a new job submission builder
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
    pub fn with_backend(mut self, backend: crate::generation::backend::BackendType) -> Self {
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

    /// Build the job submission configuration
    pub fn build(self) -> JobSubmissionConfig {
        JobSubmissionConfig {
            function: self.function,
            procedure_type: self.procedure_type,
            code_version: self.code_version,
            compute_provider: self.compute_provider,
            namespace: self.namespace,
            project: self.project,
            api_key: self.api_key,
            api_endpoint: self.api_endpoint,
            backend: self.backend,
            config_file: self.config_file,
            overrides: self.overrides,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generation::backend::BackendType;

    #[test]
    fn test_job_submission_builder() {
        let config = JobSubmissionBuilder::new(
            "test_function".to_string(),
            ProcedureType::Training,
            "v1.0.0".to_string(),
            "my-compute-provider".to_string(),
            "my-namespace".to_string(),
            "my-project".to_string(),
            "test-api-key".to_string(),
            "https://api.example.com".to_string(),
        )
        .with_backend(BackendType::Ndarray)
        .with_config_file("config.toml")
        .build();

        assert_eq!(config.function, "test_function");
        assert_eq!(config.compute_provider, "my-compute-provider");
        assert_eq!(config.backend, Some(BackendType::Ndarray));
        assert_eq!(config.config_file, Some("config.toml".to_string()));
    }

    #[test]
    fn test_job_status_equality() {
        assert_eq!(JobStatus::Submitted, JobStatus::Submitted);
        assert_ne!(JobStatus::Submitted, JobStatus::Running);
    }
}
