use std::path::PathBuf;

use crate::entity::experiments::config::ExperimentConfig;
use crate::entity::projects::burn_dir::BurnDir;
use crate::entity::projects::burn_dir::cache::CacheState;
use crate::runner::RunnerTrainingArgs;
use anyhow::Context;
use clap::Parser;
use clap::ValueHint;
use colored::Colorize;

use crate::commands::package::package_sequence;
use crate::generation::crate_gen::backend::BackendType;
use crate::print_warn;
use crate::{context::CliContext, logging::BURN_ORANGE, print_info};

/// Contains the data necessary to run an experiment.
#[derive(Debug, Clone)]
pub struct RunCommand {
    pub run_id: String,
    pub run_params: RunParams,
}

#[derive(Debug, Clone)]
pub enum RunKind {
    Training,
}

#[derive(Debug, Clone)]
pub struct RunParams {
    pub kind: RunKind,
    pub function: String,
    pub config: String,
    pub namespace: String,
    pub project: String,
    pub key: String,
}

/// Contains the data necessary to build an experiment.
#[derive(Debug)]
pub struct BuildCommand {
    pub run_id: String,
    pub backend: BackendType,
    pub code_version_digest: String,
}

fn parse_key_val(s: &str) -> Result<(String, serde_json::Value), String> {
    let (key, value) = s.split_once('=').ok_or("Must be key=value")?;
    let json_value = serde_json::from_str(value)
        .unwrap_or_else(|_| serde_json::Value::String(value.to_string()));
    Ok((key.to_string(), json_value))
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
    config: Option<String>,
    /// Batch override: e.g. --overrides a.b=3 x.y.z=true
    #[clap(long = "overrides", value_parser = parse_key_val, value_hint = ValueHint::Other, value_delimiter = ' ', num_args = 1..)]
    overrides: Vec<(String, serde_json::Value)>,
    /// Project version
    #[clap(long = "version", help = "The project version.")]
    project_version: Option<String>,
    /// The runner group name
    #[clap(long = "runner", help = "The runner group name.")]
    runner: Option<String>,
}

impl Default for TrainingArgs {
    /// Default config when running the cargo run command
    fn default() -> Self {
        Self {
            function: None,
            config: None,
            overrides: vec![],
            project_version: None,
            runner: None,
            backend: None,
        }
    }
}

pub(crate) fn handle_command(args: TrainingArgs, context: CliContext) -> anyhow::Result<()> {
    match (&args.runner, &args.project_version) {
        (Some(_), Some(_)) => Err(anyhow::anyhow!(
            "You must provide the project version to run on the runner with --version argument"
        )),
        // remote_run(args, context),
        (None, None) => local_run(args, context),
        (Some(_), None) => remote_run(args, context),
        (None, Some(_)) => {
            print_warn!(
                "Project version is ignored when executing locally (i.e. no runner is defined with --runner argument)"
            );
            local_run(args, context)
        }
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

fn remote_run(args: TrainingArgs, context: CliContext) -> anyhow::Result<()> {
    let namespace = context.get_project_path()?.owner_name;
    let project = context.get_project_path()?.project_name;

    let function = match args.function {
        Some(function) => {
            let available_functions = context.function_registry.get_training_routine();
            if !available_functions.contains(&function) {
                return Err(anyhow::anyhow!(
                    "Function `{}` is not available. Available functions are: {:?}",
                    function,
                    available_functions
                ));
            }
            function
        }
        None => prompt_function(context.function_registry.get_training_routine())?,
    };

    let code_version_digest = package_sequence(&context, false)?;
    let key = context
        .get_api_key()
        .context("Failed to get API key")?
        .to_owned();

    let command = RunnerTrainingArgs {
        function,
        backend: args.backend,
        config: args.config,
        overrides: args.overrides,
        project_version: code_version_digest.clone(),
        namespace: namespace.clone(),
        project: project.clone(),
        key,
    };

    let client = context.create_client()?;
    client.start_remote_job(
        &namespace,
        &project,
        args.runner.expect("Runner should be provided"),
        &code_version_digest,
        &serde_json::to_string(&command)?,
    )?;

    Ok(())
}

pub fn local_run_internal(
    backend: BackendType,
    config: Option<String>,
    overrides: Vec<(String, serde_json::Value)>,
    function: String,
    namespace: String,
    project: String,
    code_version_digest: String,
    key: String,
    context: &CliContext,
) -> anyhow::Result<()> {
    let kind = RunKind::Training;
    let config = ExperimentConfig::load_config(config, overrides);
    let run_id = format!("{backend}");

    let res = {
        execute_build_command(
            BuildCommand {
                run_id: run_id.clone(),
                backend,
                code_version_digest,
            },
            context,
        )?;
        execute_run_command(
            RunCommand {
                run_id: run_id.clone(),
                run_params: RunParams {
                    kind,
                    function: function.clone(),
                    config: config.data.to_string(),
                    namespace,
                    project,
                    key,
                },
            },
            context,
        )
    };

    match res {
        Ok(()) => {
            print_info!(
                "Training function `{}` executed successfully.",
                function.custom_color(BURN_ORANGE).bold()
            );
        }
        Err(e) => {
            return Err(anyhow::anyhow!(format!(
                "Failed to execute training function `{}`: {}",
                function.custom_color(BURN_ORANGE).bold(),
                e
            )));
        }
    }

    Ok(())
}

fn local_run(args: TrainingArgs, context: CliContext) -> anyhow::Result<()> {
    let namespace = context.get_project_path()?.owner_name;
    let project = context.get_project_path()?.project_name;
    let key = context
        .get_api_key()
        .context("Failed to get API key")?
        .to_owned();
    let backend = args.backend.clone().unwrap_or_default();

    let function = match args.function {
        Some(function) => {
            let available_functions = context.function_registry.get_training_routine();
            if !available_functions.contains(&function) {
                return Err(anyhow::anyhow!(
                    "Function `{}` is not available. Available functions are: {:?}",
                    function,
                    available_functions
                ));
            }
            function
        }
        None => prompt_function(context.function_registry.get_training_routine())?,
    };

    let code_version_digest = package_sequence(&context, false)?;

    local_run_internal(
        backend,
        args.config,
        args.overrides,
        function,
        namespace,
        project,
        code_version_digest,
        key,
        &context,
    )?;

    Ok(())
}

fn execute_build_command(build_command: BuildCommand, context: &CliContext) -> anyhow::Result<()> {
    print_info!(
        "Building experiment project with command: {:?}",
        build_command
    );

    generate_crate(context, &build_command)?;
    let build_status = make_build_command(&build_command, context)?.status();

    match build_status {
        Err(e) => {
            return Err(anyhow::anyhow!(format!(
                "Failed to build experiment project: {:?}",
                e
            )));
        }
        Ok(status) if !status.success() => {
            return Err(anyhow::anyhow!(format!(
                "Failed to build experiment project: {:?}",
                build_command
            )));
        }
        _ => {
            print_info!("Project built successfully.");
        }
    }

    let src_exe_path = get_target_exe_path(context);
    let target_bin_name = bin_name_from_run_id(context, &build_command.run_id);

    let burn_dir = context.burn_dir();
    let mut cache = burn_dir.load_cache().context("Failed to load cache")?;

    copy_binary(
        burn_dir,
        &mut cache,
        &target_bin_name,
        src_exe_path.to_str().unwrap(),
    )
    .context("Failed to copy binary")?;

    burn_dir.save_cache(&cache)?;

    Ok(())
}

fn execute_run_command(run_command: RunCommand, context: &CliContext) -> anyhow::Result<()> {
    print_info!("Running experiment with command: {:?}", run_command);

    let mut command = make_run_command(&run_command, context);

    let run_status = command.status();
    match run_status {
        Err(e) => {
            return Err(anyhow::anyhow!(format!(
                "Error running experiment command: {:?}",
                e
            )));
        }
        Ok(status) if !status.success() => {
            return Err(anyhow::anyhow!(format!(
                "Failed to run experiment: {:?}",
                run_command
            )));
        }
        _ => {
            print_info!("Experiment ran successfully.");
        }
    }

    Ok(())
}

fn copy_binary(
    burn_dir: &BurnDir,
    cache: &mut CacheState,
    name: &str,
    original_path: &str,
) -> std::io::Result<()> {
    let bin_path = burn_dir.bin_dir().join(name);
    std::fs::create_dir_all(burn_dir.bin_dir())?;
    std::fs::copy(original_path, &bin_path)?;

    cache.add_binary(
        name,
        bin_path.file_name().unwrap().to_string_lossy().to_string(),
    );
    Ok(())
}

fn bin_name_from_run_id(context: &CliContext, run_id: &str) -> String {
    format!(
        "{}-{}{}",
        &context.generated_crate_name(),
        run_id,
        std::env::consts::EXE_SUFFIX
    )
}

fn get_target_exe_path(context: &CliContext) -> PathBuf {
    let crate_name = &context.generated_crate_name();
    let target_path = context
        .burn_dir()
        .crates_dir()
        .join(crate_name)
        .join("target");

    target_path
        .join(&context.metadata().build_profile)
        .join(format!("{}{}", crate_name, std::env::consts::EXE_SUFFIX))
}

fn generate_crate(context: &CliContext, build_command: &BuildCommand) -> anyhow::Result<()> {
    let generated_crate = crate::generation::crate_gen::create_crate(
        context.generated_crate_name(),
        &context.metadata().user_crate_name,
        context.metadata().user_crate_dir.to_str().unwrap(),
        vec![&build_command.backend.to_string()],
        &build_command.backend,
    );

    let burn_dir = context.burn_dir();
    let mut cache = burn_dir.load_cache()?;
    generated_crate.write_to_burn_dir(burn_dir, &mut cache)?;
    burn_dir.save_cache(&cache)?;

    Ok(())
}

fn make_run_command(cmd_desc: &RunCommand, context: &CliContext) -> std::process::Command {
    let RunParams {
        kind,
        function,
        config,
        namespace,
        project,
        key,
    } = &cmd_desc.run_params;

    let kind_str = match kind {
        RunKind::Training => "train",
    };
    let bin_name = bin_name_from_run_id(context, &cmd_desc.run_id);
    let bin_exe_path = context.burn_dir().bin_dir().join(&bin_name);
    let mut command = std::process::Command::new(bin_exe_path);
    command
        .current_dir(context.cwd())
        .env("BURN_PROJECT_DIR", &context.metadata().user_crate_dir)
        .args(["--namespace", namespace])
        .args(["--project", project])
        .args(["--api-key", key])
        .args(["--endpoint", context.get_api_endpoint().as_str()])
        .args(["--config", config])
        .args([kind_str, function]);
    command
}

fn make_build_command(
    cmd_desc: &BuildCommand,
    context: &CliContext,
) -> anyhow::Result<std::process::Command> {
    let profile_arg = match context.metadata().build_profile.as_str() {
        "release" => "--release",
        "debug" => "--debug",
        _ => {
            return Err(anyhow::anyhow!(format!(
                "Invalid profile: {}",
                context.metadata().build_profile
            )));
        }
    };

    let new_target_dir: Option<String> = std::env::var("BURN_TARGET_DIR").ok();

    let mut build_command = context.cargo_cmd();
    build_command
        .arg("build")
        .arg(profile_arg)
        .arg("--no-default-features")
        .env("BURN_PROJECT_DIR", &context.metadata().user_crate_dir)
        .env(
            "BURN_CENTRAL_CODE_VERSION",
            cmd_desc.code_version_digest.as_str(),
        )
        .args([
            "--manifest-path",
            context
                .burn_dir()
                .crates_dir()
                .join(context.generated_crate_name())
                .join("Cargo.toml")
                .to_str()
                .unwrap(),
        ])
        .args(["--message-format", "short"]);
    if let Some(target_dir) = &new_target_dir {
        build_command.args(["--target-dir", target_dir]);
    }

    Ok(build_command)
}
