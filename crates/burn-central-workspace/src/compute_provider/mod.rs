use crate::{
    entity::projects::ProjectContext,
    execution::{
        BackendType, ProcedureType,
        local::{LocalExecutionConfig, LocalExecutor},
    },
};

#[derive(serde::Deserialize, serde::Serialize)]
pub struct ProcedureTypeArg {
    pub procedure_type: ProcedureType,
}

#[derive(serde::Deserialize, serde::Serialize)]
pub struct ComputeProviderJobArgs {
    /// The function to run
    pub function: String,
    /// Backend to use
    pub backend: Option<BackendType>,
    /// Config file path
    pub args: Option<serde_json::Value>,
    /// Project version/digest
    pub digest: String,
    /// Project namespace
    pub namespace: String,
    /// Project name
    pub project: String,
    /// API key
    pub key: String,
    /// API endpoint
    pub api_endpoint: String,
    /// Procedure type (training/inference)
    #[serde(flatten)]
    pub procedure_type: ProcedureTypeArg,
}

/// Main entry point for compute provider execution
pub fn compute_provider_main() -> anyhow::Result<()> {
    let manifest_path = crate::tools::cargo::try_locate_manifest().ok_or_else(|| {
        anyhow::anyhow!(
            "Could not locate Cargo.toml manifest. Please run this command inside a Burn project directory."
        )
    })?;
    let project = ProjectContext::load(&manifest_path, ".burn")?;

    let arg = get_arg()?;
    let args = serde_json::from_str::<ComputeProviderJobArgs>(&arg)?;

    execute_job(args, &project)?;

    Ok(())
}

/// Execute a job locally (this is what compute providers do - they run jobs locally)
fn execute_job(args: ComputeProviderJobArgs, project: &ProjectContext) -> anyhow::Result<()> {
    log::info!(
        "Compute Provider: Executing {} job",
        args.procedure_type.procedure_type
    );
    log::info!("Function: {}", args.function);
    log::info!("Backend: {:?}", args.backend);
    log::info!("Code version: {}", args.digest);

    let executor = LocalExecutor::new(project);

    let backend = args.backend.unwrap_or_default();

    // Build execution configuration
    let mut config = LocalExecutionConfig::new(
        args.key.clone(),
        args.api_endpoint.clone(),
        args.function.clone(),
        backend,
        args.procedure_type.procedure_type,
        args.digest,
    );

    if let Some(args) = args.args {
        config = config.with_args(args);
    }

    // Execute the job locally
    let result = executor.execute(config)?;

    if result.success {
        log::info!("Job completed successfully");
        if let Some(output) = result.output {
            log::info!("Output: {}", output);
        }
    } else {
        let error_msg = result
            .error
            .unwrap_or_else(|| "Job failed with no error message".to_string());
        log::error!("Job failed: {}", error_msg);
        anyhow::bail!("Job execution failed: {}", error_msg);
    }

    Ok(())
}

/// Get command line argument containing job parameters
fn get_arg() -> anyhow::Result<String> {
    std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("Expected exactly one argument with job parameters"))
}
