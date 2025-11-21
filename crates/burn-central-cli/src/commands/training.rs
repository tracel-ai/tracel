use std::collections::HashMap;

use anyhow::Context;
use burn_central_lib::{
    ProcedureType, ProjectContext,
    execution::parse_key_value,
    generation::backend::BackendType,
    job_submission::{JobSubmissionBuilder, JobSubmissionClient},
    local_execution::{LocalExecutionConfig, LocalExecutor},
};
use clap::Parser;
use clap::ValueHint;
use colored::Colorize;

use crate::commands::package::package_sequence;
use crate::helpers::require_linked_project;
use crate::{context::CliContext, logging::BURN_ORANGE, print_info};

fn parse_key_val(s: &str) -> Result<(String, serde_json::Value), String> {
    parse_key_value(s).map_err(|e| e.to_string())
}

#[derive(Parser, Debug)]
pub struct TrainingArgs {
    /// The training function to run. Annotate a training function with #[burn(training)] to register it.
    function: Option<String>,
    /// Backend to use
    #[clap(short = 'b', long = "backend")]
    backend: Option<BackendType>,
    /// Config file path
    #[clap(short = 'c', long = "config")]
    args: Option<String>,
    /// Batch override: e.g. --overrides a.b=3 x.y.z=true
    #[clap(long = "overrides", value_parser = parse_key_val, value_hint = ValueHint::Other, value_delimiter = ' ', num_args = 1..)]
    overrides: Vec<(String, serde_json::Value)>,
    /// Code version
    #[clap(
        long = "version",
        help = "The code version on which to run the training. (if unspecified, the current version will be packaged and used)"
    )]
    code_version: Option<String>,
    /// The compute provider group name
    #[clap(
        long = "compute-provider",
        short = 'p',
        help = "The compute provider group name."
    )]
    compute_provider: Option<String>,
}

impl Default for TrainingArgs {
    /// Default config when running the cargo run command
    fn default() -> Self {
        Self {
            function: None,
            args: None,
            overrides: vec![],
            code_version: None,
            compute_provider: None,
            backend: None,
        }
    }
}

pub(crate) fn handle_command(args: TrainingArgs, context: CliContext) -> anyhow::Result<()> {
    let project = require_linked_project(&context)?;

    match args.compute_provider {
        Some(_) => submit_job(args, &context, &project),
        None => execute_locally(args, &context, &project),
    }
}

fn prompt_function(functions: Vec<String>) -> anyhow::Result<String> {
    cliclack::select("Select the function you want to run")
        .items(
            functions
                .into_iter()
                .map(|func| (func.clone(), func.clone(), ""))
                .collect::<Vec<_>>()
                .as_slice(),
        )
        .interact()
        .map_err(anyhow::Error::from)
}

fn submit_job(
    args: TrainingArgs,
    context: &CliContext,
    project_ctx: &ProjectContext,
) -> anyhow::Result<()> {
    context
        .terminal()
        .command_title("Submit training job to platform");

    preload_functions(context, project_ctx)?;

    let bc_project = project_ctx.get_project();
    let compute_provider = args
        .compute_provider
        .context("Compute provider should be provided")?;
    let function = get_function_to_run(args.function, context, project_ctx)?;

    let code_version = match args.code_version {
        Some(version) => {
            print_info!("Using code version: {}", version);
            version
        }
        None => {
            print_info!("Packaging project and using this new code version");
            package_sequence(context, project_ctx, false)?
        }
    };

    // Create the job submission client
    let submission_client = JobSubmissionClient::new(context.core_context(), project_ctx);

    // Convert overrides to HashMap
    let overrides: HashMap<String, serde_json::Value> = args.overrides.into_iter().collect();

    // Get API key
    let api_key = context
        .core_context()
        .get_api_key()
        .context("No API key available")?;

    // Build job submission configuration
    let mut builder = JobSubmissionBuilder::new(
        function.clone(),
        ProcedureType::Training,
        code_version,
        compute_provider.clone(),
        bc_project.owner.clone(),
        bc_project.name.clone(),
        api_key.to_string(),
        context.core_context().get_api_endpoint().to_string(),
    );

    if let Some(backend) = args.backend {
        builder = builder.with_backend(backend);
    }

    if let Some(config_file) = args.args {
        builder = builder.with_config_file(config_file);
    }

    if !overrides.is_empty() {
        builder = builder.with_overrides(overrides);
    }

    let config = builder.build();

    // Submit the job
    print_info!("Submitting job to compute provider: {}", compute_provider);
    let result = submission_client.submit_job(config)?;

    if result.success {
        print_info!(
            "Training job submitted successfully for function `{}`.",
            function.custom_color(BURN_ORANGE).bold()
        );
        if let Some(job_id) = result.output {
            print_info!("Job ID: {}", job_id);
        }
    } else {
        if let Some(error) = result.error {
            return Err(anyhow::anyhow!(
                "Failed to submit training job for function `{}`: {}",
                function.custom_color(BURN_ORANGE).bold(),
                error
            ));
        } else {
            return Err(anyhow::anyhow!(
                "Failed to submit training job for function `{}`",
                function.custom_color(BURN_ORANGE).bold()
            ));
        }
    }

    Ok(())
}

fn preload_functions(context: &CliContext, project: &ProjectContext) -> anyhow::Result<()> {
    let spinner = context.terminal().spinner();
    spinner.start("Discovering project functions...");
    let functions = project.load_functions()?;
    spinner.stop(format!(
        "Discovered {} functions.",
        functions.get_function_references().len()
    ));
    Ok(())
}

fn execute_locally(
    args: TrainingArgs,
    context: &CliContext,
    project: &ProjectContext,
) -> anyhow::Result<()> {
    context.terminal().command_title("Local training execution");

    let args_json = ExperimentConfig::load_config(args.args, args.overrides)?;

    preload_functions(context, project)?;

    let function = get_function_to_run(args.function, context, project)?;

    let code_version = package_sequence(context, project, false)?;

    let executor = LocalExecutor::new(context.core_context(), project);
    let backend = args.backend.unwrap_or_default();

    // Build local execution configuration
    let config = LocalExecutionConfig::new(
        function.clone(),
        backend,
        ProcedureType::Training,
        code_version,
    )
    .with_args(args_json.data);

    // Execute locally
    print_info!("Executing training function locally: {}", function);
    let result = executor.execute(config)?;

    if result.success {
        print_info!(
            "Training function `{}` executed successfully.",
            function.custom_color(BURN_ORANGE).bold()
        );
        if let Some(output) = result.output {
            print_info!("Training output:\n{}", output);
        }
    } else {
        if let Some(error) = result.error {
            return Err(anyhow::anyhow!(
                "Failed to execute training function `{}`: {}",
                function.custom_color(BURN_ORANGE).bold(),
                error
            ));
        } else {
            return Err(anyhow::anyhow!(
                "Failed to execute training function `{}`",
                function.custom_color(BURN_ORANGE).bold()
            ));
        }
    }

    Ok(())
}

fn get_function_to_run(
    function: Option<String>,
    context: &CliContext,
    project: &ProjectContext,
) -> anyhow::Result<String> {
    // Create a local executor to get available functions
    let executor = LocalExecutor::new(context.core_context(), project);
    let available_functions = executor.list_training_functions()?;

    match function {
        Some(function) => {
            if !available_functions.contains(&function) {
                return Err(anyhow::anyhow!(
                    "Function `{}` is not available. Available functions are: {:?}",
                    function,
                    available_functions
                ));
            }
            Ok(function)
        }
        None => {
            if available_functions.is_empty() {
                return Err(anyhow::anyhow!(
                    "No training functions found in the project"
                ));
            }
            prompt_function(available_functions)
        }
    }
}

pub struct ExperimentConfig {
    pub data: serde_json::Value,
}

impl ExperimentConfig {
    fn new(value: serde_json::Value) -> Self {
        Self { data: value }
    }

    fn apply_override(&mut self, key_path: &str, value: serde_json::Value) {
        let mut parts = key_path.split('.').peekable();
        let mut target = &mut self.data;

        while let Some(part) = parts.next() {
            if parts.peek().is_none() {
                if let serde_json::Value::Object(map) = target {
                    map.insert(part.to_string(), value.clone());
                }
            } else {
                target = target
                    .as_object_mut()
                    .unwrap()
                    .entry(part)
                    .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
            }
        }
    }

    pub fn load_config(
        path: Option<String>,
        overrides: Vec<(String, serde_json::Value)>,
    ) -> anyhow::Result<Self> {
        let base_json = if let Some(path) = &path {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("failed to read config file at {}", path))?;
            serde_json::from_str(&text)
                .with_context(|| format!("failed to parse config file at {}", path))?
        } else {
            serde_json::json!({})
        };

        let mut config = ExperimentConfig::new(base_json);

        for (key, val) in &overrides {
            config.apply_override(key, val.clone());
        }

        Ok(config)
    }
}
