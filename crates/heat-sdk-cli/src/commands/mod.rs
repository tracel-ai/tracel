pub mod time;

use crate::{context::BurnCentralCliContext, generation::crate_gen::backend::BackendType, print_info};

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

/// Execute the build command for an experiment.
pub(crate) fn execute_build_command(
    build_command: BuildCommand,
    context: &mut BurnCentralCliContext,
) -> anyhow::Result<()> {
    print_info!(
        "Building experiment project with command: {:?}",
        build_command
    );

    context.generate_crate(&build_command)?;
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

    context.copy_executable_to_bin(&build_command.run_id)
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
