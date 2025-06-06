pub mod time;

use std::path::PathBuf;
use crate::{
    context::BurnCentralCliContext, generation::crate_gen::backend::BackendType, print_info,
};
use crate::burn_dir::BurnDir;
use crate::burn_dir::cache::CacheState;
use crate::generation::FileTree;

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
        config_path: String,
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
    context: &mut BurnCentralCliContext,
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

    cache.add_binary(name, bin_path.file_name().unwrap().to_string_lossy().to_string());
    Ok(())
}

fn bin_name_from_run_id(context: &BurnCentralCliContext, run_id: &str) -> String {
    format!(
        "{}-{}{}",
        &context.generated_crate_name(),
        run_id,
        std::env::consts::EXE_SUFFIX
    )
}

fn get_target_exe_path(context: &BurnCentralCliContext) -> PathBuf {
    let crate_name = &context.generated_crate_name();
    let target_path = context
        .burn_dir()
        .crates_dir()
        .join(crate_name);

    let full_path = target_path
        .join(&context.metadata().build_profile)
        .join(format!("{}{}", crate_name, std::env::consts::EXE_SUFFIX));

    full_path
}

fn generate_crate(
    context: &mut BurnCentralCliContext,
    build_command: &BuildCommand,
) -> anyhow::Result<()> {
    let generated_crate = crate::generation::crate_gen::create_crate(
        &context.generated_crate_name(),
        &context.metadata().user_project_name,
        context.metadata().user_crate_dir.to_str().unwrap(),
        vec![&build_command.backend.to_string()],
        &build_command.backend,
    );

    let burn_dir = context.burn_dir();
    let mut cache = burn_dir.load_cache()?;
    generated_crate.write_to_burn_dir(
        &burn_dir,
        &mut cache,
    )?;
    burn_dir.save_cache(&cache)?;

    Ok(())
}

/// Execute the build command for an experiment.
pub(crate) fn execute_build_command(
    build_command: BuildCommand,
    context: &mut BurnCentralCliContext,
) -> anyhow::Result<()> {
    print_info!(
        "Building experiment project with command: {:?}",
        build_command
    );

    generate_crate(context, &build_command)?;
    let build_status = context.make_build_command(&build_command)?.status();

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

    // Find the built binary path
    let src_exe_path = get_target_exe_path(context);
    let target_bin_name = bin_name_from_run_id(context, &build_command.run_id);

    let burn_dir = context.burn_dir();
    let mut cache = burn_dir.load_cache()?;

    copy_binary(
        burn_dir,
        &mut cache,
        &target_bin_name,
        src_exe_path.to_str().unwrap(),
    )?;

    burn_dir.save_cache(&cache)?;

    Ok(())
}

/// Execute the run command for an experiment.
pub(crate) fn execute_run_command(
    run_command: RunCommand,
    context: &BurnCentralCliContext,
) -> anyhow::Result<()> {
    print_info!("Running experiment with command: {:?}", run_command);

    let mut command = context.make_run_command(&run_command);

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
    mut context: BurnCentralCliContext,
) -> anyhow::Result<()> {
    for cmd in commands {
        execute_experiment_command(cmd.0, cmd.1, &mut context)?
    }

    Ok(())
}
