use anyhow::Context as _;

use crate::burn_dir::BurnDir;
use crate::burn_dir::cache::CacheState;
use crate::{context::CliContext, generation::crate_gen::backend::BackendType, print_info};
use std::path::PathBuf;

/// Contains the data necessary to run an experiment.
#[derive(Debug, Clone)]
pub struct RunCommand {
    pub run_id: String,
    pub run_params: RunParams,
}

#[derive(Debug, Clone)]
pub enum RunParams {
    Training {
        function: String,
        config: String,
        project: String,
        key: String,
    },
    // Inference
}

/// Contains the data necessary to build an experiment.
#[derive(Debug)]
pub struct BuildCommand {
    pub run_id: String,
    pub backend: BackendType,
}

/// Execute the build and run commands for an experiment.
pub(crate) fn execute_experiment_command(
    build_command: BuildCommand,
    run_command: RunCommand,
    context: &CliContext,
) -> anyhow::Result<()> {
    execute_build_command(build_command, context)?;
    execute_run_command(run_command, context)?;

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

pub fn make_build_command(
    _cmd_desc: &BuildCommand,
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
    if let Some(target_dir) = new_target_dir {
        build_command.args(["--target-dir", &target_dir]);
    }

    Ok(build_command)
}

/// Execute the build command for an experiment.
pub(crate) fn execute_build_command(
    build_command: BuildCommand,
    context: &CliContext,
) -> anyhow::Result<()> {
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

pub fn make_run_command(cmd_desc: &RunCommand, context: &CliContext) -> std::process::Command {
    match &cmd_desc.run_params {
        RunParams::Training {
            function,
            config,
            project,
            key,
        } => {
            let bin_name = bin_name_from_run_id(context, &cmd_desc.run_id);
            let bin_exe_path = context.burn_dir().bin_dir().join(&bin_name);
            let mut command = std::process::Command::new(bin_exe_path);
            command
                .current_dir(context.cwd())
                .env("BURN_PROJECT_DIR", &context.metadata().user_crate_dir)
                .args(["--project", project])
                .args(["--key", key])
                .args(["--api-endpoint", context.get_api_endpoint().as_str()])
                .args(["train", function, config]);
            command
        }
    }
}

/// Execute the run command for an experiment.
pub(crate) fn execute_run_command(
    run_command: RunCommand,
    context: &CliContext,
) -> anyhow::Result<()> {
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

/// Execute all experiments sequentially.
pub(crate) fn execute_sequentially(
    commands: Vec<(BuildCommand, RunCommand)>,
    context: CliContext,
) -> anyhow::Result<()> {
    for cmd in commands {
        execute_experiment_command(cmd.0, cmd.1, &context)?
    }

    Ok(())
}
