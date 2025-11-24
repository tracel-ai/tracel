//! Local execution core for Burn Central
//!
//! This module provides the core functionality for building and executing functions locally.
//! It is used by both the CLI (for local execution) and compute providers (for remote job execution).

use serde::Serialize;

use crate::{
    entity::projects::ProjectContext,
    execution::{BackendType, BuildProfile, ExecutionError, ProcedureType},
    tools::{cargo, function_discovery::FunctionMetadata},
};
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

/// Configuration for executing a function locally
#[derive(Debug, Clone)]
pub struct LocalExecutionConfig {
    /// The API key of the user in Burn Central
    pub api_key: String,
    /// The API endpoint to use
    pub api_endpoint: String,
    /// The function to execute
    pub function: String,
    /// Backend to use for execution
    pub backend: BackendType,
    /// Launch arguments
    pub args: serde_json::Value,
    /// Type of procedure to execute
    pub procedure_type: ProcedureType,
    /// Build profile (debug/release)
    pub build_profile: BuildProfile,
    /// Code version/digest for tracking
    pub code_version: String,
}

struct BuildConfig {
    pub backend: BackendType,
    pub build_profile: BuildProfile,
    pub code_version: String,
}

struct RunConfig {
    pub function: String,
    pub procedure_type: ProcedureType,
    pub args: serde_json::Value,
    pub api_key: String,
    pub api_endpoint: String,
}

impl LocalExecutionConfig {
    /// Create a new local execution config
    pub fn new(
        api_key: String,
        api_endpoint: String,
        function: String,
        backend: BackendType,
        procedure_type: ProcedureType,
        code_version: String,
    ) -> Self {
        Self {
            api_key,
            api_endpoint,
            function,
            backend,
            procedure_type,
            code_version,
            args: serde_json::Value::Null,
            build_profile: BuildProfile::default(),
        }
    }

    pub fn with_args<A: Serialize>(mut self, args: A) -> Self {
        self.args = serde_json::to_value(args).unwrap_or(serde_json::Value::Null);
        self
    }

    /// Set the build profile
    pub fn with_build_profile(mut self, profile: BuildProfile) -> Self {
        self.build_profile = profile;
        self
    }
}

/// Result of a local execution
#[derive(Debug)]
pub struct LocalExecutionResult {
    /// Whether the execution was successful
    pub success: bool,
    /// Output from the execution
    pub output: Option<String>,
    /// Error message if execution failed
    pub error: Option<String>,
    /// Exit code if available
    pub exit_code: Option<i32>,
}

impl LocalExecutionResult {
    /// Create a successful result
    pub fn success(output: Option<String>) -> Self {
        Self {
            success: true,
            output,
            error: None,
            exit_code: Some(0),
        }
    }

    /// Create a failed result
    pub fn failure(error: String, exit_code: Option<i32>) -> Self {
        Self {
            success: false,
            output: None,
            error: Some(error),
            exit_code,
        }
    }
}

/// Core local executor - handles building and running functions locally
pub struct LocalExecutor<'a> {
    project: &'a ProjectContext,
}

impl<'a> LocalExecutor<'a> {
    /// Create a new local executor
    pub fn new(project: &'a ProjectContext) -> Self {
        Self { project }
    }

    /// Execute a function locally
    pub fn execute(&self, config: LocalExecutionConfig) -> crate::Result<LocalExecutionResult> {
        let functions = self.project.load_functions()?;
        let function_refs = functions.get_function_references();
        self.validate_function(&config.function, function_refs)?;

        let build_config = BuildConfig {
            backend: config.backend,
            build_profile: config.build_profile,
            code_version: config.code_version,
        };

        let crate_name = "burn_central_executable";
        let crate_dir = self.generate_executable_crate(crate_name, &build_config)?;
        let executable_path = self.build_executable(crate_name, &crate_dir, &build_config)?;

        // Execute the binary
        let run_config = RunConfig {
            function: config.function,
            procedure_type: config.procedure_type,
            args: config.args,
            api_key: config.api_key,
            api_endpoint: config.api_endpoint,
        };
        self.run_executable(&executable_path, &run_config)
    }

    /// Validate that the requested function exists and matches the procedure type
    fn validate_function(
        &self,
        function: &str,
        available_functions: &[FunctionMetadata],
    ) -> crate::Result<()> {
        let function_names: Vec<&str> = available_functions
            .iter()
            .map(|f| f.routine_name.as_str())
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

    fn generate_executable_crate(
        &self,
        crate_name: &str,
        config: &BuildConfig,
    ) -> crate::Result<PathBuf> {
        let functions = self.project.load_functions()?;

        let generated_crate = crate::generation::crate_gen::create_crate(
            crate_name,
            &self.project.get_crate_name(),
            self.project.get_crate_path().to_str().unwrap(),
            &config.backend,
            functions.get_function_references(),
            self.project.get_current_package(),
        );

        let mut cache = self.project.burn_dir().load_cache()?;
        let crate_path = self.project.burn_dir().crates_dir().join(crate_name);
        generated_crate.write_to_burn_dir(&crate_path, &mut cache)?;

        Ok(crate_path)
    }

    fn build_executable(
        &self,
        crate_name: &str,
        crate_dir: &Path,
        config: &BuildConfig,
    ) -> crate::Result<PathBuf> {
        let build_dir = crate_dir;

        // Prepare cargo build command
        let mut build_cmd = cargo::command();
        build_cmd
            .current_dir(build_dir)
            .arg("build")
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());

        build_cmd.arg(config.build_profile.as_cargo_arg());

        build_cmd.env("BURN_CENTRAL_CODE_VERSION", &config.code_version);

        build_cmd.args([
            "--manifest-path",
            &build_dir.join("Cargo.toml").to_string_lossy(),
        ]);

        // Execute build
        let child = build_cmd.spawn().map_err(|e| {
            ExecutionError::BuildFailed(format!("Failed to execute cargo build: {}", e))
        })?;

        let build_output = child.wait_with_output().map_err(|e| {
            ExecutionError::BuildFailed(format!("Failed to wait for cargo build: {}", e))
        })?;
        if !build_output.status.success() {
            let stderr = String::from_utf8_lossy(&build_output.stderr);
            return Err(
                ExecutionError::BuildFailed(format!("Cargo build failed:\n{}", stderr)).into(),
            );
        }

        // Determine executable path
        let profile_dir = match config.build_profile {
            BuildProfile::Debug => "debug",
            BuildProfile::Release => "release",
        };

        let executable_name = format!("{crate_name}{}", std::env::consts::EXE_SUFFIX);
        let executable_path = build_dir
            .join("target")
            .join(profile_dir)
            .join(executable_name);

        if !executable_path.exists() {
            return Err(
                ExecutionError::BuildFailed("Built executable not found".to_string()).into(),
            );
        }

        Ok(executable_path)
    }

    /// Execute the built binary with the specified configuration
    fn run_executable(
        &self,
        executable_path: &Path,
        config: &RunConfig,
    ) -> crate::Result<LocalExecutionResult> {
        let mut run_cmd = Command::new(executable_path);

        run_cmd.env("BURN_PROJECT_DIR", &self.project.get_crate_path());

        let project = self.project.get_project();
        run_cmd.args(["--namespace", &project.owner]);
        run_cmd.args(["--project", &project.name]);

        run_cmd.args(["--api-key", &config.api_key]);

        run_cmd.args(["--endpoint", &config.api_endpoint]);

        let args_str = serde_json::to_string(&config.args).map_err(|e| {
            ExecutionError::RuntimeFailed(format!("Failed to serialize args: {}", e))
        })?;
        run_cmd.args(["--args", &args_str]);
        run_cmd.arg("train");
        run_cmd.arg(&config.function);

        // Set up stdio
        run_cmd.stdout(Stdio::inherit()).stderr(Stdio::inherit());

        // Execute
        let run_output = run_cmd.output().map_err(|e| {
            ExecutionError::RuntimeFailed(format!("Failed to execute binary: {}", e))
        })?;

        let stdout = String::from_utf8_lossy(&run_output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&run_output.stderr).to_string();

        if run_output.status.success() {
            Ok(LocalExecutionResult::success(Some(stdout)))
        } else {
            let error_message = if !stderr.is_empty() {
                stderr
            } else {
                format!(
                    "Execution failed with exit code: {:?}",
                    run_output.status.code()
                )
            };

            Ok(LocalExecutionResult::failure(
                error_message,
                run_output.status.code(),
            ))
        }
    }

    /// List available functions of a specific type
    pub fn list_functions(&self, procedure_type: ProcedureType) -> crate::Result<Vec<String>> {
        let functions = self.project.load_functions()?;
        let filtered_functions: Vec<String> = functions
            .get_function_references()
            .iter()
            .filter(|f| f.proc_type.to_lowercase() == procedure_type.to_string().to_lowercase())
            .map(|f| f.routine_name.clone())
            .collect();

        Ok(filtered_functions)
    }

    /// List all available training functions
    pub fn list_training_functions(&self) -> crate::Result<Vec<String>> {
        self.list_functions(ProcedureType::Training)
    }

    /// List all available inference functions
    pub fn list_inference_functions(&self) -> crate::Result<Vec<String>> {
        self.list_functions(ProcedureType::Inference)
    }
}
